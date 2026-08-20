from sqlmodel import Session, select

from app.config import get_settings
from app.db import init_db
from app.models import Invite
from app.runtime import prepare_environment
from app.security import generate_invite_code


def main() -> None:
    prepare_environment()
    get_settings.cache_clear()
    settings = get_settings()
    engine = init_db(settings.database_path)
    code = generate_invite_code()
    with Session(engine) as session:
        while session.exec(select(Invite).where(Invite.code == code)).first() is not None:
            code = generate_invite_code()
        session.add(Invite(code=code, created_by_id=None))
        session.commit()
    print(code)


if __name__ == "__main__":
    main()
