# GameViber community server

Quarkus service holding the modes players publish: their games, authors,
versions and downloads, the back-office at `/admin/`, and GameViber's website
(`src/main/webui-public`). One native binary, one SQLite file and the
packages beside it, one port.

---

## Prerequisites

| Tool | Version | Why |
|---|---|---|
| JDK | 21+ | build and run |
| Podman (or Docker) | — | the native build runs in a container |
| Node + npm | 20+ | the website is built at package time (`-Dsite.skip` packages without it) |

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

### The website

The site (Vue 3, `src/main/webui-public`) presents GameViber, its download and
the community's modes. In development it runs off Vite, beside `quarkus:dev`,
which it calls for `/api`:

```bash
cd src/main/webui-public && npm install && npm run dev    # http://localhost:5174
```

`./mvnw package` builds it into the static resources (`npm ci`, `npm run build`
at `prepare-package`), served at the root of the host. `SitePages` serves its
`index.html` for the site's routes reached directly (`/games`, `/games/{id}`,
`/modes/{id}`, `/m/{code}`, `/download`), with the page's title, description
and preview (OpenGraph: a mode's first capture, else its game's Steam banner)
between the `<!-- page -->` markers: search engines and the chats a link is
pasted in do not run the app. A route added to the app's router goes there too.
A mode's page opens it in the app through `gameviber://mode/<id>` or
`gameviber://shared/<code>` (`gameviber/src/links`).

Written mobile first: the styles are a phone's, `min-width` queries add the
wider screens.

---

## The API

| | Endpoint | |
|---|---|---|
| Anyone | `GET /api/games?search=` | games with public modes |
| | `GET /api/games/{id}/modes?sort=trending\|rating\|played\|new\|downloads` | a game's public modes, with their `figures` (players in 30 days, median minutes, share who came back 3 times or more, likes, Wilson rating, trend) |
| | `GET /api/games/{id}` | a game (`modes`: its public ones) |
| | `GET /api/modes/{id}`, `GET /api/modes/{id}/package?version=` | a public mode, its package (a download) |
| | `GET /api/modes/{id}/image` | its latest version's first capture (PNG), for the site and link previews |
| | `GET /api/shared/{code}`, `GET /api/shared/{code}/package`, `GET /api/shared/{code}/image` | a mode by its share code (private ones too) |
| | `POST /api/modes/{id}/reports` | `{"reason": "broken\|content\|other", "details"}` |
| | `GET /api/games/match?name=&steamAppId=` | the game a player plays, when it has public modes |
| | `POST /api/stats/plays` | `{"installation", "plays": [{"mode", "seconds", "sessions"}]}`, from players who share their stats |
| | `POST /api/stats/votes` | `{"installation", "mode", "value": 1\|-1\|0}`, once the installation played the mode |
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
image (PNG metadata is dropped), its funscripts only their actions.

---

## Production

```bash
./mvnw package -Dnative     # target/gameviber-server-0.1.0-SNAPSHOT-runner
./deploy.sh --build         # builds, uploads, swaps; --rollback puts the previous one back
```

A Debian or Ubuntu VPS is prepared once, as root, by `deploy/setup.sh`: the
`gameviber` user running the service (`gameviber-server`, systemd), the data
in `/var/lib/gameviber`, the secrets generated in `/etc/gameviber/`
(`gameviber.env`, `jwt.jwk`), and a `deploy` user whose only right is to run `/opt/gameviber/deploy.sh`:
it swaps the uploaded binary in (`releases/`, the last 5 kept), and puts the
previous one back if the new one is not ready within 30 s.

```bash
cp deploy.env.example deploy.env   # the server, the key, DEPLOY_DIR, SERVER_PORT, SERVER_ADDRESS, PROXY_ADDRESS
./deploy.sh --setup                # runs deploy/setup.sh on the server, as SETUP_USER
```

The deploy directory (`/opt/gameviber`) and the port the server listens on
(`8080`) are `DEPLOY_DIR` and `SERVER_PORT`; after changing them, run
`--setup` again. It listens on `SERVER_ADDRESS`: by default the Docker
bridge's (`172.17.0.1`) when Docker runs, for a reverse proxy in a container
(Nginx Proxy Manager), else `127.0.0.1`; for a proxy on another machine,
the server's address on their network, with the proxy's in `PROXY_ADDRESS`
(`quarkus.http.proxy.trusted-proxies`: the forwarded client address of
anyone else is ignored); never a public address. Production: `https://api.gameviber.fyustorm.ovh`,
the URL the released app is built with (`packages.yml`).

| Variable | Required | Role |
|---|---|---|
| `DATA_DIR` | yes | the database and the packages; back it up as a whole (copy it with the service stopped, or `sqlite3 gameviber.db ".backup ..."`) |
| `JWT_KEY_FILE` | yes | the HS256 key (JWK) tokens are signed with: generate one, keep it secret; the dev key is for dev and tests only |
| `ADMIN_TOKEN` | — | the back-office's token; unset, nobody administers |
| `STATS_SALT` | yes | installation ids are stored hashed with it: set one, keep it (changing it makes every player look new) |
| `LIMIT_SIGN_IN_PER_HOUR`, `LIMIT_PUBLISH_PER_HOUR`, `LIMIT_REPORT_PER_HOUR`, `LIMIT_STATS_PER_HOUR` | — | per client address: 20, 20, 10, 120 |

It sits behind a reverse proxy (HTTPS, HTTP/2, gzip, uploads of 70 MB): the
client address the limits count is the one the proxy forwards, so the proxy
must set `X-Forwarded-For` to the client's address rather than add to the
one a client sends (nginx: `proxy_set_header X-Forwarded-For $remote_addr;`).
