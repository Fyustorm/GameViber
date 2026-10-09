package fr.fyustorm.gameviber.catalog;

import java.io.IOException;
import java.io.InputStream;
import java.io.UncheckedIOException;
import java.time.Instant;
import java.util.Optional;

import org.eclipse.microprofile.config.inject.ConfigProperty;
import org.eclipse.microprofile.jwt.JsonWebToken;
import org.jboss.logging.Logger;

import fr.fyustorm.gameviber.api.Problem;
import fr.fyustorm.gameviber.api.RateLimiter;
import fr.fyustorm.gameviber.auth.Author;
import fr.fyustorm.gameviber.stats.Stats;
import io.vertx.core.http.HttpServerRequest;
import jakarta.enterprise.context.ApplicationScoped;
import jakarta.inject.Inject;
import jakarta.transaction.Transactional;
import jakarta.ws.rs.core.Response;

/** Publishing modes and their versions, and handing their packages out. */
@ApplicationScoped
public class ModeCatalog {
    private static final Logger log = Logger.getLogger(ModeCatalog.class);
    static final int MAX_NAME_CHARS = 60;
    static final int MAX_DESCRIPTION_CHARS = 2000;
    static final int MAX_CHANGELOG_CHARS = 2000;

    @Inject
    PackageStore store;
    @Inject
    Stats stats;
    @Inject
    RateLimiter limits;
    @ConfigProperty(name = "limits.code-per-hour", defaultValue = "60")
    int codePerHour;

    /** The author signed in, if they may still publish. */
    public Author author(JsonWebToken jwt) {
        Author author = Author.<Author>findByIdOptional(Long.parseLong(jwt.getSubject()))
                .orElseThrow(() -> Problem.of(Response.Status.UNAUTHORIZED, "this author is gone"));
        if (author.blocked) {
            throw Problem.forbidden("this author can no longer publish");
        }
        return author;
    }

    public Views.ModeDetail detail(Mode mode, boolean owner) {
        return Views.detail(mode, owner, stats.figures(mode.id));
    }

    /** A public mode, by its public id. */
    public static Mode publicMode(String id) {
        return Mode.byPublicId(id).filter(Mode::isPublic).orElseThrow(() -> Problem.notFound("no such mode"));
    }

    /** A mode (private ones too) by its share code; codes are guessed only a few at a time. */
    public Mode byCode(String code, HttpServerRequest request) {
        limits.check("code", request, codePerHour);
        return Mode.byShareCode(code).filter(m -> m.withdrawnAt == null).orElseThrow(() -> Problem.notFound("no mode has this code"));
    }

    /** The first capture of its latest version, if it has one. */
    public Optional<byte[]> cover(Mode mode) {
        var latest = ModeVersion.latest(mode);
        if (mode.withdrawnAt != null || latest.isEmpty()) {
            return Optional.empty();
        }
        try {
            return store.cover(mode, latest.get().number);
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
    }

    /** A mode of `author`, by its public id (withdrawn ones too). */
    public Mode own(Author author, String publicId) {
        Mode mode = Mode.byPublicId(publicId).orElseThrow(() -> Problem.notFound("no such mode"));
        if (!mode.authorId.equals(author.id)) {
            throw Problem.forbidden("this mode is someone else's");
        }
        return mode;
    }

    /** A new mode, its first version from `upload`. */
    @Transactional
    public Mode publish(Author author, InputStream upload, String name, String description, String visibility, String changelog) {
        SharedPackage pkg = check(upload);
        var mode = new Mode();
        mode.publicId = Codes.publicId();
        mode.gameId = Game.findOrCreate(pkg.gameName, pkg.steamAppId).id;
        mode.authorId = author.id;
        mode.name = text(name, "a name", MAX_NAME_CHARS, true);
        mode.description = text(description, "a description", MAX_DESCRIPTION_CHARS, false);
        mode.visibility = visibility(visibility);
        mode.shareCode = Codes.shareCode();
        mode.createdAt = mode.updatedAt = Instant.now();
        mode.persist();
        addVersion(mode, pkg, changelog, 1);
        log.infof("%s published %s (%s)", author.pseudo, mode.name, mode.publicId);
        return mode;
    }

    /** The next version of `mode`, for the same game. */
    @Transactional
    public Mode publishVersion(Mode mode, InputStream upload, String changelog) {
        if (mode.withdrawnAt != null) {
            throw Problem.badRequest("this mode was withdrawn");
        }
        SharedPackage pkg = check(upload);
        Game game = Game.findById(mode.gameId);
        boolean sameGame = pkg.steamAppId != null && pkg.steamAppId.equals(game.steamAppId) || Game.slug(pkg.gameName).equals(game.slug);
        if (!sameGame) {
            throw Problem.badRequest("this version is for another game (" + pkg.gameName + ") than " + game.name);
        }
        int number = ModeVersion.latest(mode).map(v -> v.number + 1).orElse(1);
        addVersion(mode, pkg, changelog, number);
        mode.updatedAt = Instant.now();
        log.infof("%s: version %d", mode.publicId, number);
        return mode;
    }

    private void addVersion(Mode mode, SharedPackage pkg, String changelog, int number) {
        try {
            store.save(mode, number, pkg.bytes);
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        var version = new ModeVersion();
        version.modeId = mode.id;
        version.number = number;
        version.changelog = text(changelog, "a changelog", MAX_CHANGELOG_CHARS, false);
        version.api = pkg.api;
        version.size = pkg.bytes.length;
        version.sha256 = pkg.sha256;
        version.createdAt = Instant.now();
        version.persist();
    }

    /** The package of a version (the latest when `number` is null), counted as a download. */
    @Transactional
    public Download.Served download(Mode mode, Integer number) {
        if (mode.withdrawnAt != null) {
            throw Problem.notFound("this mode was withdrawn");
        }
        Optional<ModeVersion> version = number == null
                ? ModeVersion.latest(mode)
                : ModeVersion.find("modeId = ?1 and number = ?2", mode.id, number).firstResultOptional();
        ModeVersion v = version.orElseThrow(() -> Problem.notFound("no such version"));
        Mode.update("downloads = downloads + 1 where id = ?1", mode.id);
        var download = new Download();
        download.modeId = mode.id;
        download.versionNumber = v.number;
        download.createdAt = Instant.now();
        download.persist();
        return new Download.Served(store.path(mode, v.number), mode.publicId + "-" + v.number + ".gameviber");
    }

    private static SharedPackage check(InputStream upload) {
        if (upload == null) {
            throw Problem.badRequest("the mode's file is missing");
        }
        try {
            return SharedPackage.check(upload);
        } catch (SharedPackage.Invalid e) {
            throw Problem.badRequest(e.getMessage());
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
    }

    static String visibility(String visibility) {
        if (visibility == null || visibility.isBlank()) {
            return Mode.PRIVATE;
        }
        if (!visibility.equals(Mode.PRIVATE) && !visibility.equals(Mode.PUBLIC)) {
            throw Problem.badRequest("visibility is private or public");
        }
        return visibility;
    }

    static String text(String text, String what, int max, boolean required) {
        String value = text == null ? "" : text.trim();
        if (required && value.isEmpty()) {
            throw Problem.badRequest(what + " is needed");
        }
        if (value.length() > max) {
            throw Problem.badRequest(what + " is " + max + " characters at most");
        }
        return value;
    }
}
