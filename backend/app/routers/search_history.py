from fastapi import APIRouter, Depends, HTTPException, Response
from sqlalchemy import delete
from sqlalchemy.dialects.sqlite import insert
from sqlmodel import Session, col, select

from app.deps import get_current_user, get_session
from app.models import SearchHistory, User, utcnow
from app.schemas import SearchHistoryIn, SearchHistoryOut

router = APIRouter(prefix="/api/search-history", tags=["search-history"])
LIMIT = 20


def _entries(session: Session, user_id: int) -> list[SearchHistory]:
    return list(session.exec(
        select(SearchHistory)
        .where(SearchHistory.user_id == user_id)
        .order_by(col(SearchHistory.last_used_at).desc(), col(SearchHistory.id).desc())
    ).all())


@router.get("", response_model=list[SearchHistoryOut])
def get_history(
    me: User = Depends(get_current_user), session: Session = Depends(get_session),
):
    return _entries(session, me.id)


@router.post("", response_model=list[SearchHistoryOut])
def remember_search(
    payload: SearchHistoryIn,
    me: User = Depends(get_current_user), session: Session = Depends(get_session),
):
    now = utcnow()
    # One write transaction covers deduplication and pruning, including parallel tabs.
    statement = insert(SearchHistory).values(
        user_id=me.id, query=payload.query,
        normalized_query=payload.query.casefold(), last_used_at=now,
    )
    session.execute(statement.on_conflict_do_update(
        index_elements=["user_id", "normalized_query"],
        set_={"query": payload.query, "last_used_at": now},
    ))
    entries = _entries(session, me.id)
    for entry in entries[LIMIT:]:
        session.delete(entry)
    session.commit()
    return entries[:LIMIT]


@router.delete("", status_code=204)
def clear_history(
    me: User = Depends(get_current_user), session: Session = Depends(get_session),
):
    session.execute(delete(SearchHistory).where(SearchHistory.user_id == me.id))
    session.commit()
    return Response(status_code=204)


@router.delete("/{entry_id}", status_code=204)
def delete_entry(
    entry_id: int,
    me: User = Depends(get_current_user), session: Session = Depends(get_session),
):
    entry = session.get(SearchHistory, entry_id)
    if entry is None or entry.user_id != me.id:
        raise HTTPException(status_code=404, detail="Search history entry not found")
    session.delete(entry)
    session.commit()
    return Response(status_code=204)
