package fr.fyustorm.gameviber.catalog;

import java.util.Comparator;
import java.util.List;
import java.util.Optional;

import fr.fyustorm.gameviber.api.Problem;
import fr.fyustorm.gameviber.stats.Stats;
import io.vertx.core.http.HttpServerRequest;
import jakarta.inject.Inject;
import jakarta.ws.rs.DefaultValue;
import jakarta.ws.rs.GET;
import jakarta.ws.rs.Path;
import jakarta.ws.rs.PathParam;
import jakarta.ws.rs.QueryParam;
import jakarta.ws.rs.core.Context;
import jakarta.ws.rs.core.Response;

/** What anyone reads: the games and their public modes; a private mode by its code (`ModeResource`: a mode). */
@Path("/api")
public class CatalogResource {
    @Inject
    ModeCatalog catalog;
    @Inject
    Stats stats;
    @Inject
    PackageStore store;

    /** Games with public modes, the most modes first; `search` in their name. */
    @GET
    @Path("/games")
    public List<Views.GameView> games(@QueryParam("search") String search) {
        String slug = search == null ? "" : Game.slug(search);
        return Game.<Game>list("from Game g where exists (from Mode m where m.gameId = g.id and m.visibility = ?1 and m.withdrawnAt is null)", Mode.PUBLIC)
                .stream()
                .filter(g -> g.slug.contains(slug))
                .map(Views::game)
                .sorted((a, b) -> Long.compare(b.modes(), a.modes()))
                .toList();
    }

    /** A game, even without public modes (`modes` is 0 then). */
    @GET
    @Path("/games/{id}")
    public Views.GameView game(@PathParam("id") long id) {
        return Game.<Game>findByIdOptional(id).map(Views::game).orElseThrow(() -> Problem.notFound("no such game"));
    }

    /**
     * The game a player plays, as GameViber knows it (its Steam app id, else its
     * name), when it has public modes: what the app tells the player about.
     */
    @GET
    @Path("/games/match")
    public Views.GameView match(@QueryParam("name") String name, @QueryParam("steamAppId") Long steamAppId) {
        var byApp = steamAppId == null ? Optional.<Game>empty() : Game.<Game>find("steamAppId", steamAppId).firstResultOptional();
        var game = byApp.or(() -> name == null || name.isBlank() ? Optional.empty() : Game.<Game>find("slug", Game.slug(name)).firstResultOptional());
        return game.map(Views::game).filter(g -> g.modes() > 0).orElseThrow(() -> Problem.notFound("no public mode for this game"));
    }

    /**
     * A game, and its public modes, `sort`ed: trending (default: played lately,
     * the latest days counting more), rating (the most liked, few votes
     * counting little), played (players in the last 30 days), new, downloads.
     */
    @GET
    @Path("/games/{id}/modes")
    public List<Views.ModeSummary> modes(@PathParam("id") long id, @QueryParam("sort") @DefaultValue("trending") String sort) {
        Game.<Game>findByIdOptional(id).orElseThrow(() -> Problem.notFound("no such game"));
        Comparator<Views.ModeSummary> order = switch (sort) {
            // As sure as likes are, then the most played (no vote yet).
            case "rating" -> Comparator.<Views.ModeSummary>comparingDouble(m -> -m.figures().rating()).thenComparingLong(m -> -m.figures().players());
            case "played" -> Comparator.comparingLong(m -> -m.figures().players());
            case "downloads" -> Comparator.comparingLong(m -> -m.downloads());
            case "new" -> Comparator.comparing(Views.ModeSummary::updatedAt).reversed();
            default -> Comparator.comparingDouble(m -> -m.figures().trend());
        };
        return Mode.<Mode>list("gameId = ?1 and visibility = ?2 and withdrawnAt is null", id, Mode.PUBLIC)
                .stream()
                .map(m -> Views.summary(m, stats.figures(m.id), store))
                .sorted(order.thenComparing(Comparator.comparing(Views.ModeSummary::updatedAt).reversed()))
                .toList();
    }

    /** A private mode (or a public one) shared by its code. */
    @GET
    @Path("/shared/{code}")
    public Views.ModeDetail shared(@PathParam("code") String code, @Context HttpServerRequest request) {
        return catalog.detail(catalog.byCode(code, request), false);
    }

    @GET
    @Path("/shared/{code}/package")
    public Response sharedDownload(@PathParam("code") String code, @QueryParam("version") Integer version, @Context HttpServerRequest request) {
        return send(catalog.download(catalog.byCode(code, request), version));
    }

    @GET
    @Path("/shared/{code}/image")
    public Response sharedImage(@PathParam("code") String code, @Context HttpServerRequest request) {
        return image(catalog.cover(catalog.byCode(code, request)));
    }

    @GET
    @Path("/shared/{code}/captures/{file}")
    public Response sharedCapture(@PathParam("code") String code, @PathParam("file") String file, @Context HttpServerRequest request) {
        return image(catalog.sharedCapture(code, file, request));
    }

    /** A mode's cover, kept an hour by browsers and link previews. */
    static Response image(Optional<byte[]> png) {
        return Response.ok(png.orElseThrow(() -> Problem.notFound("this mode has no image")), "image/png")
                .header("Cache-Control", "public, max-age=3600")
                .build();
    }

    static Response send(Download.Served served) {
        return Response.ok(served.path().toFile(), "application/zip")
                .header("Content-Disposition", "attachment; filename=\"" + served.fileName() + "\"")
                .build();
    }
}
