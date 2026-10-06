# GameViber community server

Quarkus service holding the modes players publish: their games, authors,
versions and downloads, and the back-office at `/admin/`. One native binary,
one SQLite file and the packages beside it, one port.

---

## Prerequisites

| Tool | Version | Why |
|---|---|---|
| JDK | 21+ | build and run |
| Podman (or Docker) | — | the native build runs in a container |

Maven comes with the wrapper (`./mvnw`).

---

## Dev mode

```bash
./mvnw quarkus:dev
```

- API: <http://localhost:8080/api/...>, its description at <http://localhost:8080/q/swagger-ui>
- Back-office: <http://localhost:8080/admin/>, token `dev`
- Data: `data/` (the SQLite database and `packages/`); Liquibase migrates it at start

`./mvnw test` runs the tests against a database of their own, in `target/test-data/`.

---

## The API

| | Endpoint | |
|---|---|---|
| Anyone | `GET /api/games?search=` | games with public modes |
| | `GET /api/games/{id}/modes?sort=new\|downloads` | a game's public modes |
| | `GET /api/modes/{id}`, `GET /api/modes/{id}/package?version=` | a public mode, its package (a download) |
| | `GET /api/shared/{code}`, `GET /api/shared/{code}/package` | a mode by its share code (private ones too) |
| | `POST /api/modes/{id}/reports` | `{"reason": "broken\|content\|other", "details"}` |
| | `POST /api/authors`, `POST /api/authors/session` | `{"pseudo", "password"}` → `{"token", "pseudo"}` |
| Author (`Authorization: Bearer <token>`) | `POST /api/modes` | multipart: `package` (a `.gameviber` file), `name`, `description`, `visibility` (`private` by default, or `public`), `changelog` |
| | `POST /api/modes/{id}/versions` | multipart: `package`, `changelog` |
| | `PATCH /api/modes/{id}` | `{"name", "description", "visibility"}` |
| | `POST /api/modes/{id}/share-code`, `DELETE /api/modes/{id}` | a new share code; withdraw |
| | `GET /api/authors/me/modes` | |
| Administrator | `POST /api/admin/session` | form `token` → an 8-hour session |
| | `/api/admin/modes`, `/authors`, `/reports`, `/games/{id}/merge?into=` | the back-office's |

Errors answer `{"error": "<a sentence to show>"}`. A package is checked as
GameViber checks one it imports (`SharedPackage`): its entries, their sizes,
the format, the mode API its scripts declare; its captures keep only their
image (PNG metadata is dropped).

---

## Production

```bash
./mvnw package -Dnative     # target/gameviber-server-0.1.0-SNAPSHOT-runner
./deploy.sh                 # SSH_HOST=<server> ./deploy.sh --build
```

| Variable | Required | Role |
|---|---|---|
| `DATA_DIR` | yes | the database and the packages; back it up as a whole (copy it with the service stopped, or `sqlite3 gameviber.db ".backup ..."`) |
| `JWT_KEY_FILE` | yes | the HS256 key (JWK) tokens are signed with: generate one, keep it secret; the dev key is for dev and tests only |
| `ADMIN_TOKEN` | — | the back-office's token; unset, nobody administers |
| `LIMIT_SIGN_IN_PER_HOUR`, `LIMIT_PUBLISH_PER_HOUR`, `LIMIT_REPORT_PER_HOUR` | — | per client address: 20, 20, 10 |

It sits behind a reverse proxy (HTTPS, HTTP/2, gzip): the client address the
limits count is the one the proxy forwards.
