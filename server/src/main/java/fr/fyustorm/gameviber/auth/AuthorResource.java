package fr.fyustorm.gameviber.auth;

import java.time.Instant;
import java.util.List;
import java.util.regex.Pattern;

import org.eclipse.microprofile.config.inject.ConfigProperty;
import org.eclipse.microprofile.jwt.JsonWebToken;

import fr.fyustorm.gameviber.api.Problem;
import fr.fyustorm.gameviber.api.RateLimiter;
import fr.fyustorm.gameviber.catalog.Mode;
import fr.fyustorm.gameviber.catalog.ModeCatalog;
import fr.fyustorm.gameviber.catalog.Views;
import io.quarkus.elytron.security.common.BcryptUtil;
import io.vertx.core.http.HttpServerRequest;
import jakarta.annotation.security.RolesAllowed;
import jakarta.inject.Inject;
import jakarta.transaction.Transactional;
import jakarta.ws.rs.GET;
import jakarta.ws.rs.POST;
import jakarta.ws.rs.Path;
import jakarta.ws.rs.core.Context;
import jakarta.ws.rs.core.Response;

/**
 * Authors: a pseudo and a password, nothing else asked (no email: a lost
 * password cannot be recovered until accounts come, lot 5 of the plan).
 */
@Path("/api/authors")
public class AuthorResource {
    private static final Pattern PSEUDO = Pattern.compile("[\\p{L}\\p{N}_\\-. ]{3,24}");
    private static final int MIN_PASSWORD = 8;

    @Inject
    Tokens tokens;
    @Inject
    RateLimiter limits;
    @Inject
    JsonWebToken jwt;
    @Inject
    ModeCatalog catalog;
    @ConfigProperty(name = "limits.sign-in-per-hour")
    int signInPerHour;

    public record Credentials(String pseudo, String password) {}

    public record Session(String token, String pseudo) {}

    @POST
    @Transactional
    public Session register(Credentials credentials, @Context HttpServerRequest request) {
        limits.check("sign-in", request, signInPerHour);
        String pseudo = credentials == null || credentials.pseudo() == null ? "" : credentials.pseudo().trim();
        String password = credentials == null || credentials.password() == null ? "" : credentials.password();
        if (!PSEUDO.matcher(pseudo).matches()) {
            throw Problem.badRequest("a pseudo is 3 to 24 letters, digits, spaces, dots, dashes or underscores");
        }
        if (password.length() < MIN_PASSWORD) {
            throw Problem.badRequest("a password has at least " + MIN_PASSWORD + " characters");
        }
        if (Author.byPseudo(pseudo).isPresent()) {
            throw Problem.of(Response.Status.CONFLICT, "this pseudo is taken");
        }
        var author = new Author();
        author.pseudo = pseudo;
        author.pseudoKey = Author.key(pseudo);
        author.passwordHash = BcryptUtil.bcryptHash(password);
        author.createdAt = Instant.now();
        author.persist();
        return new Session(tokens.author(author), author.pseudo);
    }

    @POST
    @Path("/session")
    public Session signIn(Credentials credentials, @Context HttpServerRequest request) {
        limits.check("sign-in", request, signInPerHour);
        String pseudo = credentials == null || credentials.pseudo() == null ? "" : credentials.pseudo();
        String password = credentials == null || credentials.password() == null ? "" : credentials.password();
        Author author = Author.byPseudo(pseudo)
                .filter(a -> BcryptUtil.matches(password, a.passwordHash))
                .orElseThrow(() -> Problem.of(Response.Status.UNAUTHORIZED, "wrong pseudo or password"));
        if (author.blocked) {
            throw Problem.forbidden("this author can no longer publish");
        }
        return new Session(tokens.author(author), author.pseudo);
    }

    /** The modes of the author signed in, withdrawn ones included. */
    @GET
    @Path("/me/modes")
    @RolesAllowed(Tokens.AUTHOR)
    public List<Views.ModeDetail> myModes() {
        Author author = catalog.author(jwt);
        return Mode.<Mode>list("authorId = ?1 order by updatedAt desc", author.id).stream().map(m -> catalog.detail(m, true)).toList();
    }
}
