package fr.fyustorm.gameviber.auth;

import java.time.Duration;
import java.util.Set;

import org.eclipse.microprofile.config.inject.ConfigProperty;

import io.smallrye.jwt.build.Jwt;
import jakarta.enterprise.context.ApplicationScoped;

/** The JWTs the server hands out: an author's (its id as subject), an administrator's. */
@ApplicationScoped
public class Tokens {
    public static final String AUTHOR = "author";
    public static final String ADMIN = "admin";

    @ConfigProperty(name = "mp.jwt.verify.issuer")
    String issuer;
    @ConfigProperty(name = "author.session-duration")
    Duration authorSession;
    @ConfigProperty(name = "admin.session-duration")
    Duration adminSession;

    public String author(Author author) {
        return Jwt.issuer(issuer).subject(author.id.toString()).upn(author.pseudo).groups(Set.of(AUTHOR)).expiresIn(authorSession).sign();
    }

    /** Not numeric: it names no author. */
    public String admin() {
        return Jwt.issuer(issuer).subject(ADMIN).upn(ADMIN).groups(Set.of(ADMIN)).expiresIn(adminSession).sign();
    }
}
