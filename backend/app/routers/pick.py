from fastapi import APIRouter, BackgroundTasks, Depends, HTTPException, Request
from fastapi.responses import Response
from sqlalchemy.engine import Engine
from sqlalchemy.orm import selectinload
from sqlmodel import Session, col, select

from app import activity, push
from app.branding import sanitize_name
from app.ics import build_meeting_ics
from app.config import get_settings
from app.deps import get_current_user, get_session
from app.models import (
    ClubPick,
    ClubPickPost,
    PickMilestone,
    PostReaction,
    ShelfEntry,
    ShelfStatus,
    User,
    utcnow,
)
from app.pick_ops import apply_club_pick, current_pick, pick_out
from app.schemas import (
    ClubPickCurrentOut,
    ClubPickHistoryOut,
    ClubPickOut,
    ClubPickSetIn,
    PickPostIn,
    PickPostListOut,
    PickPostOut,
    PickPostPatchIn,
    ReactionIn,
    ReactionOut,
)
from app.timezone import meeting_label, resolved_timezone

router = APIRouter(prefix="/api/pick", tags=["pick"])


def _timezone() -> str:
    return resolved_timezone(get_settings().bookclub_tz)


def _current_pick(session: Session) -> ClubPick | None:
    return current_pick(session)


def _pick_out(session: Session, pick: ClubPick, viewer: User) -> ClubPickOut:
    return pick_out(session, pick, viewer, _timezone())


def _club_name() -> str:
    return sanitize_name(get_settings().bookclub_name)


def notify_new_pick(
    background: BackgroundTasks | None,
    engine: Engine,
    session: Session,
    pick: ClubPick,
    actor: User,
) -> None:
    if pick.book is None:
        return
    push.schedule(
        background,
        engine,
        session,
        kind="pick",
        title=f"{_club_name()}: new club pick",
        body=f"{actor.username} chose {pick.book.title}.",
        url="/",
        tag=f"pick-{pick.id}",
        exclude_user_id=actor.id,
    )


@router.get("", response_model=ClubPickCurrentOut)
def get_club_pick(
    request: Request,
    background: BackgroundTasks,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> ClubPickCurrentOut:
    pick = _current_pick(session)
    if pick is None:
        return ClubPickCurrentOut(pick=None, timezone=_timezone())
    push.maybe_schedule_meeting_reminder(
        background, request.app.state.engine, session, pick, _club_name()
    )
    return ClubPickCurrentOut(pick=_pick_out(session, pick, me), timezone=_timezone())


@router.get("/history", response_model=ClubPickHistoryOut)
def club_pick_history(
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> ClubPickHistoryOut:
    rows = session.exec(
        select(ClubPick)
        .where(ClubPick.ended_at.is_not(None))
        .options(selectinload(ClubPick.book), selectinload(ClubPick.set_by))
        .order_by(col(ClubPick.ended_at).desc(), ClubPick.id.desc())
    ).all()
    return ClubPickHistoryOut(
        items=[_pick_out(session, row, me) for row in rows],
        timezone=_timezone(),
    )


@router.put("", response_model=ClubPickOut)
def set_club_pick(
    payload: ClubPickSetIn,
    request: Request,
    background: BackgroundTasks,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> ClubPickOut:
    before = _current_pick(session)
    before_id = before.id if before is not None else None
    loaded = apply_club_pick(session, me, payload, _timezone())
    if loaded.id != before_id:
        notify_new_pick(background, request.app.state.engine, session, loaded, me)
    return _pick_out(session, loaded, me)


@router.delete("", response_model=ClubPickCurrentOut)
def clear_club_pick(
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> ClubPickCurrentOut:
    current = _current_pick(session)
    if current is None:
        return ClubPickCurrentOut(pick=None, timezone=_timezone())
    current.ended_at = utcnow()
    session.add(current)
    activity.record(session, me, "pick_cleared", book=current.book, pick=current)
    session.commit()
    return ClubPickCurrentOut(pick=None, timezone=_timezone())


def _load_pick(session: Session, pick_id: int) -> ClubPick | None:
    return session.exec(
        select(ClubPick)
        .where(ClubPick.id == pick_id)
        .options(selectinload(ClubPick.book), selectinload(ClubPick.set_by))
    ).first()


def _post_out(post: ClubPickPost, viewer: User, tz_name: str) -> PickPostOut:
    author = post.author.username if post.author is not None else "unknown"
    grouped: dict[str, list[str]] = {}
    for reaction in post.reactions:
        name = reaction.user.username if reaction.user is not None else "unknown"
        grouped.setdefault(reaction.emoji, []).append(name)
    reactions = [
        ReactionOut(
            emoji=emoji,
            count=len(users),
            mine=viewer.username in users,
            users=sorted(users),
        )
        for emoji, users in grouped.items()
    ]
    reactions.sort(key=lambda row: (-row.count, row.emoji))
    return PickPostOut(
        id=post.id or 0,
        author=author,
        mine=post.author_id == viewer.id,
        body=post.body,
        spoiler_upto=post.spoiler_upto,
        milestone_id=post.milestone_id,
        created_at=post.created_at,
        created_label=meeting_label(post.created_at, tz_name) or "",
        edited=post.updated_at is not None,
        reactions=reactions,
    )


def _post_query():
    return select(ClubPickPost).options(
        selectinload(ClubPickPost.author),
        selectinload(ClubPickPost.reactions).selectinload(PostReaction.user),
    )


def _load_post(session: Session, post_id: int) -> ClubPickPost | None:
    return session.exec(_post_query().where(ClubPickPost.id == post_id)).first()


def _viewer_entry(session: Session, pick: ClubPick, viewer: User) -> ShelfEntry | None:
    return session.exec(
        select(ShelfEntry).where(
            ShelfEntry.user_id == viewer.id, ShelfEntry.book_id == pick.book_id
        )
    ).first()


def _posts_for(session: Session, pick: ClubPick, viewer: User) -> PickPostListOut:
    tz_name = _timezone()
    rows = session.exec(
        _post_query()
        .where(ClubPickPost.pick_id == pick.id)
        .order_by(ClubPickPost.created_at, ClubPickPost.id)
    ).all()
    mine = _viewer_entry(session, pick, viewer)
    my_progress: int | None = None
    if mine is not None:
        if mine.status == ShelfStatus.finished:
            my_progress = 100
        elif mine.status == ShelfStatus.currently_reading:
            my_progress = mine.progress
    return PickPostListOut(
        pick_id=pick.id or 0,
        can_post=pick.ended_at is None,
        my_progress=my_progress,
        my_status=mine.status if mine is not None else None,
        items=[_post_out(row, viewer, tz_name) for row in rows],
        timezone=tz_name,
    )


def _create_post(
    session: Session,
    pick: ClubPick,
    me: User,
    payload: PickPostIn,
    *,
    background: BackgroundTasks | None = None,
    engine: Engine | None = None,
) -> PickPostOut:
    if pick.ended_at is not None:
        raise HTTPException(status_code=400, detail="That pick is closed")
    if payload.milestone_id is not None:
        milestone = session.get(PickMilestone, payload.milestone_id)
        if milestone is None or milestone.pick_id != pick.id:
            raise HTTPException(status_code=400, detail="That milestone is not on this pick")
    post = ClubPickPost(
        pick_id=pick.id or 0,
        author_id=me.id or 0,
        body=payload.body,
        spoiler_upto=payload.spoiler_upto,
        milestone_id=payload.milestone_id,
    )
    session.add(post)
    session.flush()
    activity.record(session, me, "note_posted", book=pick.book, pick=pick, post_id=post.id)
    session.commit()
    loaded = _load_post(session, post.id or 0)
    if loaded is None:
        raise HTTPException(status_code=500, detail="Could not save that note")
    if engine is not None and pick.book is not None:
        excerpt = payload.body if len(payload.body) <= 90 else payload.body[:87] + "…"
        push.schedule(
            background,
            engine,
            session,
            kind="note",
            title=f"{me.username} on {pick.book.title}",
            body=excerpt,
            url="/",
            tag=f"note-{pick.id}",
            exclude_user_id=me.id,
        )
    return _post_out(loaded, me, _timezone())


@router.get("/posts", response_model=PickPostListOut)
def current_pick_posts(
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> PickPostListOut:
    pick = _current_pick(session)
    if pick is None:
        raise HTTPException(status_code=404, detail="No club pick yet")
    return _posts_for(session, pick, me)


@router.post("/posts", response_model=PickPostOut, status_code=201)
def add_current_pick_post(
    payload: PickPostIn,
    request: Request,
    background: BackgroundTasks,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> PickPostOut:
    pick = _current_pick(session)
    if pick is None:
        raise HTTPException(status_code=404, detail="No club pick yet")
    return _create_post(
        session, pick, me, payload, background=background, engine=request.app.state.engine
    )


@router.patch("/posts/{post_id}", response_model=PickPostOut)
def edit_post(
    post_id: int,
    payload: PickPostPatchIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> PickPostOut:
    post = _load_post(session, post_id)
    if post is None:
        raise HTTPException(status_code=404, detail="No note with that id")
    if post.author_id != me.id:
        raise HTTPException(status_code=403, detail="You can only edit your own notes")
    if payload.body is None and "spoiler_upto" not in payload.model_fields_set:
        raise HTTPException(status_code=400, detail="Nothing to update")
    if payload.body is not None:
        post.body = payload.body
    if "spoiler_upto" in payload.model_fields_set:
        post.spoiler_upto = payload.spoiler_upto
    post.updated_at = utcnow()
    session.add(post)
    session.commit()
    loaded = _load_post(session, post_id)
    if loaded is None:
        raise HTTPException(status_code=404, detail="No note with that id")
    return _post_out(loaded, me, _timezone())


@router.delete("/posts/{post_id}", status_code=204)
def delete_post(
    post_id: int,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> None:
    post = session.get(ClubPickPost, post_id)
    if post is None:
        raise HTTPException(status_code=404, detail="No note with that id")
    if post.author_id != me.id:
        raise HTTPException(status_code=403, detail="You can only delete your own notes")
    for reaction in session.exec(
        select(PostReaction).where(PostReaction.post_id == post_id)
    ).all():
        session.delete(reaction)
    session.delete(post)
    session.commit()


@router.post("/posts/{post_id}/reactions", response_model=PickPostOut)
def toggle_reaction(
    post_id: int,
    payload: ReactionIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> PickPostOut:
    post = _load_post(session, post_id)
    if post is None:
        raise HTTPException(status_code=404, detail="No note with that id")
    existing = session.exec(
        select(PostReaction).where(
            PostReaction.post_id == post_id,
            PostReaction.user_id == me.id,
            PostReaction.emoji == payload.emoji,
        )
    ).first()
    if existing is not None:
        session.delete(existing)
    else:
        session.add(PostReaction(post_id=post_id, user_id=me.id or 0, emoji=payload.emoji))
        pick = _load_pick(session, post.pick_id)
        activity.record(
            session,
            me,
            "reaction",
            book=pick.book if pick else None,
            pick=pick,
            emoji=payload.emoji,
            post_id=post_id,
            author=post.author.username if post.author else None,
        )
    session.commit()
    session.expire_all()
    loaded = _load_post(session, post_id)
    if loaded is None:
        raise HTTPException(status_code=404, detail="No note with that id")
    return _post_out(loaded, me, _timezone())


@router.get("/meeting.ics")
def meeting_ics(
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> Response:
    pick = _current_pick(session)
    if pick is None or pick.meeting_at is None or pick.book is None:
        raise HTTPException(status_code=404, detail="No meeting scheduled")
    settings = get_settings()
    body = build_meeting_ics(
        pick_id=pick.id or 0,
        club_name=sanitize_name(settings.bookclub_name),
        title=pick.book.title,
        authors=pick.book.authors,
        note=pick.note,
        starts_at=pick.meeting_at,
        ol_work_key=pick.book.ol_work_key,
        public_url=settings.bookclub_public_url,
    )
    filename = f"bookclub-{pick.id}.ics"
    return Response(
        content=body,
        media_type="text/calendar; charset=utf-8",
        headers={"Content-Disposition": f'attachment; filename="{filename}"'},
    )


@router.get("/{pick_id}/posts", response_model=PickPostListOut)
def pick_posts(
    pick_id: int,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> PickPostListOut:
    pick = _load_pick(session, pick_id)
    if pick is None:
        raise HTTPException(status_code=404, detail="No club pick with that id")
    return _posts_for(session, pick, me)


@router.post("/{pick_id}/posts", response_model=PickPostOut, status_code=201)
def add_pick_post(
    pick_id: int,
    payload: PickPostIn,
    request: Request,
    background: BackgroundTasks,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> PickPostOut:
    pick = _load_pick(session, pick_id)
    if pick is None:
        raise HTTPException(status_code=404, detail="No club pick with that id")
    return _create_post(
        session, pick, me, payload, background=background, engine=request.app.state.engine
    )

