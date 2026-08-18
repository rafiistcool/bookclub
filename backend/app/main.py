import logging
from pathlib import Path

from fastapi import FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse, JSONResponse
from fastapi.staticfiles import StaticFiles
from starlette.middleware.sessions import SessionMiddleware

from app.config import get_settings
from app.db import ensure_bootstrap_invite, init_db
from app.routers import auth, books, invites, members, shelf
from sqlmodel import Session

STATIC_DIR = Path(__file__).resolve().parent / "static"
logger = logging.getLogger("bookclub")


def create_app() -> FastAPI:
    settings = get_settings()
    if not settings.debug and settings.secret_key == "dev-secret-change-me":
        raise RuntimeError("SECRET_KEY must be set when DEBUG=0")

    engine = init_db(settings.database_path)
    with Session(engine) as session:
        ensure_bootstrap_invite(session, settings)

    app = FastAPI(
        title="Bookclub",
        docs_url="/api/docs" if settings.debug else None,
        redoc_url=None,
        openapi_url="/api/openapi.json" if settings.debug else None,
    )
    app.state.engine = engine
    app.state.settings = settings

    app.add_middleware(
        SessionMiddleware,
        secret_key=settings.secret_key,
        session_cookie="bookclub_session",
        max_age=30 * 24 * 3600,
        same_site="lax",
        https_only=settings.bookclub_https,
    )
    if settings.debug:
        app.add_middleware(
            CORSMiddleware,
            allow_origins=[
                "http://localhost:5173",
                "http://127.0.0.1:5173",
            ],
            allow_credentials=True,
            allow_methods=["*"],
            allow_headers=["*"],
        )

    @app.exception_handler(RequestValidationError)
    async def validation_handler(
        _request: Request, exc: RequestValidationError
    ) -> JSONResponse:
        messages = []
        for error in exc.errors():
            msg = str(error.get("msg", "Invalid value"))
            if msg.startswith("Value error, "):
                msg = msg[len("Value error, ") :]
            messages.append(msg)
        return JSONResponse(status_code=400, content={"detail": "; ".join(messages)})

    @app.get("/api/health")
    def health() -> dict[str, bool]:
        return {"ok": True}

    app.include_router(auth.router)
    app.include_router(invites.router)
    app.include_router(books.router)
    app.include_router(shelf.router)
    app.include_router(members.router)

    if STATIC_DIR.is_dir():
        assets = STATIC_DIR / "assets"
        if assets.is_dir():
            app.mount("/assets", StaticFiles(directory=assets), name="assets")

        @app.get("/{full_path:path}")
        def spa(full_path: str):
            if full_path.startswith("api/"):
                return JSONResponse(status_code=404, content={"detail": "Not found"})
            candidate = (STATIC_DIR / full_path).resolve()
            if candidate.is_file() and STATIC_DIR in candidate.parents:
                return FileResponse(candidate)
            index = STATIC_DIR / "index.html"
            if index.is_file():
                return FileResponse(index)
            return JSONResponse(status_code=404, content={"detail": "Not found"})

    return app


app = create_app()
