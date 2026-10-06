package fr.fyustorm.gameviber.stats;

import java.util.List;
import java.util.regex.Pattern;

import org.eclipse.microprofile.config.inject.ConfigProperty;

import fr.fyustorm.gameviber.api.Problem;
import fr.fyustorm.gameviber.api.RateLimiter;
import fr.fyustorm.gameviber.catalog.Mode;
import io.vertx.core.http.HttpServerRequest;
import jakarta.inject.Inject;
import jakarta.transaction.Transactional;
import jakarta.ws.rs.POST;
import jakarta.ws.rs.Path;
import jakarta.ws.rs.core.Context;
import jakarta.ws.rs.core.Response;

/**
 * What the app sends when the player shares their stats: play time with the
 * modes they installed, and their votes. No account: an installation id.
 */
@Path("/api/stats")
public class StatsResource {
    /** A random id the app made: 16 to 64 letters, digits and dashes. */
    private static final Pattern INSTALLATION = Pattern.compile("[A-Za-z0-9-]{16,64}");
    private static final int MAX_PLAYS = 50;

    @Inject
    Stats stats;
    @Inject
    RateLimiter limits;
    @ConfigProperty(name = "limits.stats-per-hour")
    int statsPerHour;

    public record Play(String mode, long seconds, int sessions) {}

    public record Plays(String installation, List<Play> plays) {}

    /** Play time since the last report; modes unknown or withdrawn are skipped. */
    @POST
    @Path("/plays")
    @Transactional
    public Response plays(Plays report, @Context HttpServerRequest request) {
        limits.check("stats", request, statsPerHour);
        String installation = installation(report == null ? null : report.installation());
        List<Play> plays = report.plays() == null ? List.of() : report.plays();
        if (plays.size() > MAX_PLAYS) {
            throw Problem.badRequest("at most " + MAX_PLAYS + " modes a report");
        }
        for (Play play : plays) {
            if (play == null || play.mode() == null) {
                continue;
            }
            Mode.byPublicId(play.mode()).filter(m -> m.withdrawnAt == null)
                    .ifPresent(mode -> stats.play(mode.id, installation, play.seconds(), play.sessions()));
        }
        return Response.noContent().build();
    }

    public record Vote(String installation, String mode, int value) {}

    /** 1 liked, -1 not, 0 taken back; only after playing the mode. */
    @POST
    @Path("/votes")
    @Transactional
    public Response vote(Vote vote, @Context HttpServerRequest request) {
        limits.check("stats", request, statsPerHour);
        String installation = installation(vote == null ? null : vote.installation());
        if (vote.value() < -1 || vote.value() > 1) {
            throw Problem.badRequest("a vote is 1, -1 or 0");
        }
        Mode mode = Mode.byPublicId(vote.mode() == null ? "" : vote.mode()).filter(m -> m.withdrawnAt == null)
                .orElseThrow(() -> Problem.notFound("no such mode"));
        if (!stats.played(mode.id, installation)) {
            throw Problem.forbidden("play it first, then say what you think of it");
        }
        stats.vote(mode.id, installation, vote.value());
        return Response.noContent().build();
    }

    private String installation(String id) {
        if (id == null || !INSTALLATION.matcher(id).matches()) {
            throw Problem.badRequest("an installation id is 16 to 64 letters, digits or dashes");
        }
        return stats.installation(id);
    }
}
