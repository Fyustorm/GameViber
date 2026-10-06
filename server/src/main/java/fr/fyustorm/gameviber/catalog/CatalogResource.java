package fr.fyustorm.gameviber.catalog;

import java.util.List;
import java.util.Optional;

import org.eclipse.microprofile.config.inject.ConfigProperty;

import fr.fyustorm.gameviber.api.Problem;
import fr.fyustorm.gameviber.api.RateLimiter;
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
    RateLimiter limits;
    @ConfigProperty(name = "limits.code-per-hour", defaultValue = "60")
    int codePerHour;

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

    /** A game, and its public modes: `sort` new (default) or downloads. */
    @GET
    @Path("/games/{id}/modes")
    public List<Views.ModeSummary> modes(@PathParam("id") long id, @QueryParam("sort") @DefaultValue("new") String sort) {
        Game.<Game>findByIdOptional(id).orElseThrow(() -> Problem.notFound("no such game"));
        String order = sort.equals("downloads") ? "downloads desc, updatedAt desc" : "updatedAt desc";
        return Mode.<Mode>list("gameId = ?1 and visibility = ?2 and withdrawnAt is null order by " + order, id, Mode.PUBLIC)
                .stream()
                .map(Views::summary)
                .toList();
    }

    /** A private mode (or a public one) shared by its code. */
    @GET
    @Path("/shared/{code}")
    public Views.ModeDetail shared(@PathParam("code") String code, @Context HttpServerRequest request) {
        return catalog.detail(byCode(code, request), false);
    }

    @GET
    @Path("/shared/{code}/package")
    public Response sharedDownload(@PathParam("code") String code, @QueryParam("version") Integer version, @Context HttpServerRequest request) {
        return send(catalog.download(byCode(code, request), version));
    }

    /** Codes are guessed only a few at a time. */
    private Mode byCode(String code, HttpServerRequest request) {
        limits.check("code", request, codePerHour);
        return Mode.byShareCode(code).filter(m -> m.withdrawnAt == null).orElseThrow(() -> Problem.notFound("no mode has this code"));
    }

    static Response send(Download.Served served) {
        return Response.ok(served.path().toFile(), "application/zip")
                .header("Content-Disposition", "attachment; filename=\"" + served.fileName() + "\"")
                .build();
    }
}
