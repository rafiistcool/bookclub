from datetime import datetime

from fastapi import APIRouter, BackgroundTasks, Depends, HTTPException, Request
from pydantic import BaseModel, field_validator
from sqlmodel import Session, select

from app import push
from app.branding import sanitize_name
from app.config import get_settings
from app.deps import get_current_user, get_session
from app.models import PushSubscription, User

router = APIRouter(prefix="/api/push", tags=["push"])


class VapidOut(BaseModel):
    public_key: str


class SubscriptionKeys(BaseModel):
    p256dh: str
    auth: str


class SubscribeIn(BaseModel):
    endpoint: str
    keys: SubscriptionKeys
    user_agent: str = ""

    @field_validator("endpoint")
    @classmethod
    def endpoint_ok(cls, value: str) -> str:
        value = value.strip()
        if not value.startswith("https://") or len(value) > 1024:
            raise ValueError("Push endpoint must be an https URL")
        return value


class UnsubscribeIn(BaseModel):
    endpoint: str


class SubscriptionOut(BaseModel):
    id: int
    endpoint_tail: str
    user_agent: str
    created_at: datetime


class SubscriptionListOut(BaseModel):
    items: list[SubscriptionOut]


class TestOut(BaseModel):
    sent_to: int


@router.get("/vapid", response_model=VapidOut)
def vapid_public_key(me: User = Depends(get_current_user)) -> VapidOut:
    return VapidOut(public_key=push.public_key_b64url())


@router.get("/subscriptions", response_model=SubscriptionListOut)
def my_subscriptions(
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> SubscriptionListOut:
    rows = session.exec(
        select(PushSubscription)
        .where(PushSubscription.user_id == me.id)
        .order_by(PushSubscription.id)
    ).all()
    return SubscriptionListOut(
        items=[
            SubscriptionOut(
                id=row.id or 0,
                endpoint_tail=row.endpoint[-12:],
                user_agent=row.user_agent,
                created_at=row.created_at,
            )
            for row in rows
        ]
    )


@router.post("/subscriptions", status_code=201, response_model=SubscriptionOut)
def subscribe(
    payload: SubscribeIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> SubscriptionOut:
    existing = session.exec(
        select(PushSubscription).where(PushSubscription.endpoint == payload.endpoint)
    ).first()
    if existing is not None:
        # A device re-subscribing (or another account on the same browser)
        # takes over the endpoint.
        existing.user_id = me.id or 0
        existing.p256dh = payload.keys.p256dh
        existing.auth = payload.keys.auth
        existing.user_agent = payload.user_agent[:256]
        row = existing
    else:
        row = PushSubscription(
            user_id=me.id or 0,
            endpoint=payload.endpoint,
            p256dh=payload.keys.p256dh,
            auth=payload.keys.auth,
            user_agent=payload.user_agent[:256],
        )
    session.add(row)
    session.commit()
    session.refresh(row)
    return SubscriptionOut(
        id=row.id or 0,
        endpoint_tail=row.endpoint[-12:],
        user_agent=row.user_agent,
        created_at=row.created_at,
    )


@router.delete("/subscriptions", status_code=204)
def unsubscribe(
    payload: UnsubscribeIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> None:
    row = session.exec(
        select(PushSubscription).where(
            PushSubscription.endpoint == payload.endpoint.strip(),
            PushSubscription.user_id == me.id,
        )
    ).first()
    if row is None:
        raise HTTPException(status_code=404, detail="No such subscription")
    session.delete(row)
    session.commit()


@router.post("/test", response_model=TestOut)
def send_test(
    request: Request,
    background: BackgroundTasks,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> TestOut:
    rows = session.exec(
        select(PushSubscription).where(PushSubscription.user_id == me.id)
    ).all()
    if not rows:
        raise HTTPException(status_code=400, detail="This device is not subscribed yet")
    targets = [
        {"id": row.id, "endpoint": row.endpoint, "p256dh": row.p256dh, "auth": row.auth}
        for row in rows
    ]
    club = sanitize_name(get_settings().bookclub_name)
    background.add_task(
        push.deliver,
        request.app.state.engine,
        targets,
        {
            "title": f"{club}: notifications are on",
            "body": "You will hear about new picks, notes, and meeting reminders.",
            "url": "/settings",
            "tag": "test",
            "kind": "test",
        },
    )
    return TestOut(sent_to=len(targets))
