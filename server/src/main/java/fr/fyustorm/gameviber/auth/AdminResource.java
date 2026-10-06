package fr.fyustorm.gameviber.auth;

import java.security.MessageDigest;
import java.nio.charset.StandardCharsets;
import java.time.Instant;
import java.util.List;

import org.eclipse.microprofile.config.inject.ConfigProperty;
import org.jboss.logging.Logger;

import fr.fyustorm.gameviber.api.Problem;
import fr.fyustorm.gameviber.api.RateLimiter;
import fr.fyustorm.gameviber.catalog.Game;
import fr.fyustorm.gameviber.catalog.Mode;
import fr.fyustorm.gameviber.catalog.ModeCatalog;
import fr.fyustorm.gameviber.catalog.Report;
import fr.fyustorm.gameviber.catalog.Views;
import io.vertx.core.http.HttpServerRequest;
import jakarta.annotation.security.PermitAll;
import jakarta.annotation.security.RolesAllowed;
import jakarta.inject.Inject;
import jakarta.transaction.Transactional;
import jakarta.ws.rs.Consumes;
import jakarta.ws.rs.FormParam;
import jakarta.ws.rs.GET;
import jakarta.ws.rs.POST;
import jakarta.ws.rs.Path;
import jakarta.ws.rs.PathParam;
import jakarta.ws.rs.QueryParam;
import jakarta.ws.rs.core.Context;
import jakarta.ws.rs.core.MediaType;
import jakarta.ws.rs.core.Response;

/**
 * The back-office: the operations token (ADMIN_TOKEN) is traded once for an
 * administration session, which withdraws modes, blocks authors, merges games
 * and handles reports.
 */
@Path("/api/admin")
@RolesAllowed(Tokens.ADMIN)
public class AdminResource {
    private static final Logger log = Logger.getLogger(AdminResource.class);

    @Inject
    Tokens tokens;
    @Inject
    RateLimiter limits;
    @Inject
    ModeCatalog catalog;
    @ConfigProperty(name = "admin.token")
    String adminToken;
    @ConfigProperty(name = "limits.sign-in-per-hour")
    int signInPerHour;

    public record Session(String token) {}

    @POST
    @Path("/session")
    @PermitAll
    @Consumes(MediaType.APPLICATION_FORM_URLENCODED)
    public Session signIn(@FormParam("token") String token, @Context HttpServerRequest request) {
        limits.check("admin-sign-in", request, signInPerHour);
        if (adminToken == null || adminToken.isBlank()) {
            throw Problem.of(Response.Status.SERVICE_UNAVAILABLE, "no administrator here (ADMIN_TOKEN is not set)");
        }
        byte[] given = (token == null ? "" : token).getBytes(StandardCharsets.UTF_8);
        if (!MessageDigest.isEqual(given, adminToken.getBytes(StandardCharsets.UTF_8))) {
            throw Problem.of(Response.Status.UNAUTHORIZED, "wrong token");
        }
        log.info("administration session opened");
        return new Session(tokens.admin());
    }

    /** Every mode, newest first; `withdrawn` true: only withdrawn ones. */
    @GET
    @Path("/modes")
    public List<Views.ModeDetail> modes(@QueryParam("withdrawn") boolean withdrawn) {
        String query = withdrawn ? "withdrawnAt is not null order by updatedAt desc" : "order by updatedAt desc";
        return Mode.<Mode>list(query).stream().map(m -> catalog.detail(m, true)).toList();
    }

    @POST
    @Path("/modes/{id}/withdraw")
    @Consumes(MediaType.APPLICATION_FORM_URLENCODED)
    @Transactional
    public Views.ModeDetail withdraw(@PathParam("id") String id, @FormParam("reason") String reason) {
        Mode mode = mode(id);
        mode.withdrawnAt = Instant.now();
        mode.withdrawnReason = reason == null || reason.isBlank() ? "withdrawn by an administrator" : reason.trim();
        log.infof("%s withdrawn: %s", mode.publicId, mode.withdrawnReason);
        return catalog.detail(mode, true);
    }

    @POST
    @Path("/modes/{id}/restore")
    @Transactional
    public Views.ModeDetail restore(@PathParam("id") String id) {
        Mode mode = mode(id);
        mode.withdrawnAt = null;
        mode.withdrawnReason = null;
        return catalog.detail(mode, true);
    }

    public record AuthorView(long id, String pseudo, boolean blocked, long modes, Instant createdAt) {}

    @GET
    @Path("/authors")
    public List<AuthorView> authors() {
        return Author.<Author>list("order by createdAt desc").stream()
                .map(a -> new AuthorView(a.id, a.pseudo, a.blocked, Mode.count("authorId", a.id), a.createdAt))
                .toList();
    }

    /** Blocked: they can no longer sign in nor publish (their modes stay, withdraw them apart). */
    @POST
    @Path("/authors/{id}/block")
    @Transactional
    public AuthorView block(@PathParam("id") long id, @QueryParam("blocked") @jakarta.ws.rs.DefaultValue("true") boolean blocked) {
        Author author = Author.<Author>findByIdOptional(id).orElseThrow(() -> Problem.notFound("no such author"));
        author.blocked = blocked;
        return new AuthorView(author.id, author.pseudo, author.blocked, Mode.count("authorId", author.id), author.createdAt);
    }

    /** Two names of one game: `id`'s modes join `into`, which keeps its name. */
    @POST
    @Path("/games/{id}/merge")
    @Transactional
    public Views.GameView merge(@PathParam("id") long id, @QueryParam("into") long into) {
        Game from = Game.<Game>findByIdOptional(id).orElseThrow(() -> Problem.notFound("no such game"));
        Game to = Game.<Game>findByIdOptional(into).orElseThrow(() -> Problem.notFound("no such game"));
        if (from.id.equals(to.id)) {
            throw Problem.badRequest("a game cannot be merged into itself");
        }
        Mode.update("gameId = ?1 where gameId = ?2", to.id, from.id);
        if (to.steamAppId == null && from.steamAppId != null) {
            Long app = from.steamAppId;
            from.delete();
            Game.flush();
            to.steamAppId = app;
        } else {
            from.delete();
        }
        log.infof("game %s merged into %s", from.name, to.name);
        return new Views.GameView(to.id, to.name, to.steamAppId, Mode.count("gameId", to.id));
    }

    public record ReportView(long id, String mode, String modeName, String reason, String details, Instant createdAt, Instant resolvedAt) {}

    /** Reports, open ones first. */
    @GET
    @Path("/reports")
    public List<ReportView> reports(@QueryParam("all") boolean all) {
        String query = all ? "order by resolvedAt nulls first, createdAt desc" : "resolvedAt is null order by createdAt desc";
        return Report.<Report>list(query).stream().map(r -> {
            Mode mode = Mode.findById(r.modeId);
            return new ReportView(r.id, mode.publicId, mode.name, r.reason, r.details, r.createdAt, r.resolvedAt);
        }).toList();
    }

    @POST
    @Path("/reports/{id}/resolve")
    @Transactional
    public Response resolve(@PathParam("id") long id) {
        Report report = Report.<Report>findByIdOptional(id).orElseThrow(() -> Problem.notFound("no such report"));
        report.resolvedAt = Instant.now();
        return Response.noContent().build();
    }

    private static Mode mode(String id) {
        return Mode.byPublicId(id).orElseThrow(() -> Problem.notFound("no such mode"));
    }
}
