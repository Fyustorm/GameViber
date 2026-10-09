package fr.fyustorm.gameviber.site;

import java.io.IOException;
import java.io.InputStream;
import java.io.UncheckedIOException;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.Optional;

import fr.fyustorm.gameviber.catalog.Game;
import fr.fyustorm.gameviber.catalog.Mode;
import fr.fyustorm.gameviber.catalog.ModeCatalog;
import fr.fyustorm.gameviber.catalog.Views;
import io.vertx.core.http.HttpServerRequest;
import jakarta.inject.Inject;
import jakarta.ws.rs.GET;
import jakarta.ws.rs.Path;
import jakarta.ws.rs.PathParam;
import jakarta.ws.rs.WebApplicationException;
import jakarta.ws.rs.core.Context;
import jakarta.ws.rs.core.MediaType;
import jakarta.ws.rs.core.Response;
import jakarta.ws.rs.core.UriInfo;

/**
 * The site's pages reached directly (a link, a reload): its `index.html`, the
 * Vue app (`src/main/webui-public`) built into the static resources, with the
 * page's title and its preview (OpenGraph) for search engines and the chats a
 * link is pasted in, which do not run the app. The home page is the static file.
 */
@Path("/")
public class SitePages {
    static final String SITE = "GameViber";
    static final String DEFAULT_TITLE = "GameViber: feel your games on your toys";
    static final String DEFAULT_DESCRIPTION = "GameViber turns the rumble, the sound and the image of your games into vibrations"
            + " for the toys connected to Intiface Central. Free and open source, for Linux.";
    static final int MAX_DESCRIPTION_CHARS = 200;
    /** What the built index.html has between them is the page's: replaced here. */
    static final String START = "<!-- page -->";
    static final String END = "<!-- /page -->";
    /** Without the built site (`quarkus:dev`, tests): the app runs off Vite then. */
    static final String UNBUILT = """
            <!doctype html>
            <html lang="en">
            <head>
            <meta charset="UTF-8">
            <!-- page -->
            <!-- /page -->
            </head>
            <body><p>The site is not built: in development it runs off Vite, <a href="http://localhost:5174">localhost:5174</a>.</p></body>
            </html>
            """;

    private static volatile String template;

    @Inject
    ModeCatalog catalog;
    @Context
    UriInfo uri;

    /** What a page says about itself; `image` is a path on this site or an absolute URL. */
    record Page(String title, String description, String image, boolean index, Response.Status status) {
        static Page of(String title, String description, String image) {
            return new Page(title, description, image, true, Response.Status.OK);
        }

        static Page notFound() {
            return new Page("Not found", DEFAULT_DESCRIPTION, null, false, Response.Status.NOT_FOUND);
        }
    }

    @GET
    @Path("games")
    public Response games() {
        return send(Page.of("Community modes", "Modes other GameViber players made and shared, by game.", null));
    }

    @GET
    @Path("games/{id}")
    public Response game(@PathParam("id") String id) {
        Optional<Game> game = parseId(id).flatMap(i -> Game.<Game>findByIdOptional(i));
        return send(game.map(g -> {
            long modes = Views.game(g).modes();
            String description = modes == 0
                    ? "No public GameViber mode for " + g.name + " yet."
                    : modes + (modes == 1 ? " GameViber mode" : " GameViber modes") + " for " + g.name + ", made by players: feel the game on your toys.";
            return Page.of(g.name + " modes", description, steamHeader(g));
        }).orElseGet(Page::notFound));
    }

    @GET
    @Path("modes/{id}")
    public Response mode(@PathParam("id") String id) {
        try {
            Mode mode = ModeCatalog.publicMode(id);
            return send(modePage(mode, "/api/modes/" + mode.publicId + "/image", true));
        } catch (WebApplicationException e) {
            return send(Page.notFound());
        }
    }

    /** A mode shared by its code: private ones are not indexed. */
    @GET
    @Path("m/{code}")
    public Response shared(@PathParam("code") String code, @Context HttpServerRequest request) {
        try {
            Mode mode = catalog.byCode(code, request);
            return send(modePage(mode, "/api/shared/" + mode.shareCode + "/image", false));
        } catch (WebApplicationException e) {
            return send(Page.notFound());
        }
    }

    @GET
    @Path("download")
    public Response download() {
        return send(Page.of("Download", "Download GameViber for Linux: packages for Ubuntu, Debian, Fedora, Arch, and an archive for SteamOS and other systems.", null));
    }

    private Page modePage(Mode mode, String cover, boolean index) {
        Game game = Game.findById(mode.gameId);
        String description = mode.description == null || mode.description.isBlank()
                ? "A GameViber mode for " + game.name + ": feel the game on your toys."
                : shorten(mode.description);
        String image = catalog.cover(mode).isPresent() ? cover : steamHeader(game);
        return new Page(mode.name + " for " + game.name, description, image, index, Response.Status.OK);
    }

    private Response send(Page page) {
        return Response.status(page.status())
                .type(MediaType.TEXT_HTML_TYPE.withCharset("utf-8"))
                // Always asked again: a new release changes the assets it names.
                .header("Cache-Control", "no-cache")
                .entity(render(template(), page, uri.getBaseUri(), uri.getPath()))
                .build();
    }

    /** `template` with the page's head between its markers; `base` and `path` make its absolute URLs. */
    static String render(String template, Page page, URI base, String path) {
        String title = page.title() == null ? DEFAULT_TITLE : page.title() + " · " + SITE;
        String image = page.image() == null ? "/og.jpg" : page.image();
        var head = new StringBuilder();
        head.append("<title>").append(escape(title)).append("</title>\n");
        head.append(meta("name", "description", page.description()));
        if (!page.index()) {
            head.append(meta("name", "robots", "noindex"));
        }
        head.append(meta("property", "og:type", "website"));
        head.append(meta("property", "og:site_name", SITE));
        head.append(meta("property", "og:title", page.title() == null ? DEFAULT_TITLE : page.title()));
        head.append(meta("property", "og:description", page.description()));
        head.append(meta("property", "og:url", base.resolve(path.replaceFirst("^/", "")).toString()));
        head.append(meta("property", "og:image", base.resolve(image.replaceFirst("^/", "")).toString()));
        head.append(meta("name", "twitter:card", "summary_large_image"));
        int start = template.indexOf(START);
        int end = template.indexOf(END);
        if (start < 0 || end < start) {
            throw new IllegalStateException("index.html has no " + START + " ... " + END + " block");
        }
        return template.substring(0, start + START.length()) + "\n" + head + template.substring(end);
    }

    static String shorten(String text) {
        String flat = text.strip().replaceAll("\\s+", " ");
        if (flat.length() <= MAX_DESCRIPTION_CHARS) {
            return flat;
        }
        String cut = flat.substring(0, MAX_DESCRIPTION_CHARS - 1);
        int space = cut.lastIndexOf(' ');
        return (space > MAX_DESCRIPTION_CHARS / 2 ? cut.substring(0, space) : cut) + "…";
    }

    static String steamHeader(Game game) {
        return game.steamAppId == null ? null
                : "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/" + game.steamAppId + "/header.jpg";
    }

    private static String meta(String attribute, String name, String content) {
        return "<meta " + attribute + "=\"" + name + "\" content=\"" + escape(content) + "\" />\n";
    }

    static String escape(String text) {
        return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace("\"", "&quot;").replace("'", "&#39;");
    }

    private static Optional<Long> parseId(String id) {
        try {
            return Optional.of(Long.parseLong(id));
        } catch (NumberFormatException e) {
            return Optional.empty();
        }
    }

    /** The built index.html, read once. */
    private static String template() {
        if (template == null) {
            try (InputStream in = Thread.currentThread().getContextClassLoader().getResourceAsStream("META-INF/resources/index.html")) {
                template = in == null ? UNBUILT : new String(in.readAllBytes(), StandardCharsets.UTF_8);
            } catch (IOException e) {
                throw new UncheckedIOException(e);
            }
        }
        return template;
    }
}
