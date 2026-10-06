package fr.fyustorm.gameviber.catalog;

import java.io.IOException;
import java.io.InputStream;
import java.io.UncheckedIOException;
import java.nio.file.Files;
import java.time.Instant;

import org.eclipse.microprofile.config.inject.ConfigProperty;
import org.eclipse.microprofile.jwt.JsonWebToken;
import org.jboss.resteasy.reactive.RestForm;
import org.jboss.resteasy.reactive.multipart.FileUpload;

import fr.fyustorm.gameviber.api.Problem;
import fr.fyustorm.gameviber.api.RateLimiter;
import fr.fyustorm.gameviber.auth.Author;
import fr.fyustorm.gameviber.auth.Tokens;
import io.vertx.core.http.HttpServerRequest;
import jakarta.annotation.security.PermitAll;
import jakarta.annotation.security.RolesAllowed;
import jakarta.inject.Inject;
import jakarta.transaction.Transactional;
import jakarta.ws.rs.Consumes;
import jakarta.ws.rs.DELETE;
import jakarta.ws.rs.GET;
import jakarta.ws.rs.PATCH;
import jakarta.ws.rs.POST;
import jakarta.ws.rs.Path;
import jakarta.ws.rs.PathParam;
import jakarta.ws.rs.QueryParam;
import jakarta.ws.rs.core.Context;
import jakarta.ws.rs.core.MediaType;
import jakarta.ws.rs.core.Response;

/**
 * A mode: what anyone reads (a public one, its package) and reports, and what
 * its author does with it: publish it, its new versions, change or withdraw it.
 */
@Path("/api/modes")
@RolesAllowed(Tokens.AUTHOR)
public class ModeResource {
    @Inject
    ModeCatalog catalog;
    @Inject
    RateLimiter limits;
    @Inject
    JsonWebToken jwt;
    @ConfigProperty(name = "limits.publish-per-hour")
    int publishPerHour;
    @ConfigProperty(name = "limits.report-per-hour")
    int reportPerHour;

    @GET
    @PermitAll
    @Path("/{id}")
    public Views.ModeDetail mode(@PathParam("id") String id) {
        return catalog.detail(publicMode(id), false);
    }

    /** Its latest version's package, or version `version`'s. */
    @GET
    @PermitAll
    @Path("/{id}/package")
    public Response download(@PathParam("id") String id, @QueryParam("version") Integer version) {
        return CatalogResource.send(catalog.download(publicMode(id), version));
    }

    public record ReportForm(String reason, String details) {}

    @POST
    @PermitAll
    @Path("/{id}/reports")
    @Transactional
    public Response report(@PathParam("id") String id, ReportForm form, @Context HttpServerRequest request) {
        limits.check("report", request, reportPerHour);
        Mode mode = Mode.byPublicId(id).filter(m -> m.withdrawnAt == null).orElseThrow(() -> Problem.notFound("no such mode"));
        if (form == null || !Report.REASONS.contains(form.reason())) {
            throw Problem.badRequest("a reason is broken, content or other");
        }
        var report = new Report();
        report.modeId = mode.id;
        report.reason = form.reason();
        report.details = ModeCatalog.text(form.details(), "the details", 1000, false);
        report.createdAt = Instant.now();
        report.persist();
        return Response.noContent().build();
    }


    /** A new mode: `package` its `.gameviber` file, `visibility` private (default) or public. */
    @POST
    @Consumes(MediaType.MULTIPART_FORM_DATA)
    public Views.ModeDetail publish(
            @RestForm("package") FileUpload file,
            @RestForm String name,
            @RestForm String description,
            @RestForm String visibility,
            @RestForm String changelog,
            @Context HttpServerRequest request) {
        limits.check("publish", request, publishPerHour);
        Author author = catalog.author(jwt);
        try (InputStream upload = open(file)) {
            return catalog.detail(catalog.publish(author, upload, name, description, visibility, changelog), true);
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
    }

    @POST
    @Path("/{id}/versions")
    @Consumes(MediaType.MULTIPART_FORM_DATA)
    public Views.ModeDetail publishVersion(
            @PathParam("id") String id, @RestForm("package") FileUpload file, @RestForm String changelog, @Context HttpServerRequest request) {
        limits.check("publish", request, publishPerHour);
        Mode mode = catalog.own(catalog.author(jwt), id);
        try (InputStream upload = open(file)) {
            return catalog.detail(catalog.publishVersion(mode, upload, changelog), true);
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
    }

    public record Changes(String name, String description, String visibility) {}

    /** Its name, description or visibility (what is null stays). */
    @PATCH
    @Path("/{id}")
    @Transactional
    public Views.ModeDetail change(@PathParam("id") String id, Changes changes) {
        Mode mode = catalog.own(catalog.author(jwt), id);
        if (changes != null) {
            if (changes.name() != null) {
                mode.name = ModeCatalog.text(changes.name(), "a name", ModeCatalog.MAX_NAME_CHARS, true);
            }
            if (changes.description() != null) {
                mode.description = ModeCatalog.text(changes.description(), "a description", ModeCatalog.MAX_DESCRIPTION_CHARS, false);
            }
            if (changes.visibility() != null) {
                mode.visibility = ModeCatalog.visibility(changes.visibility());
            }
            mode.updatedAt = Instant.now();
        }
        return catalog.detail(mode, true);
    }

    /** A new share code: the old one no longer works. */
    @POST
    @Path("/{id}/share-code")
    @Transactional
    public Views.ModeDetail newShareCode(@PathParam("id") String id) {
        Mode mode = catalog.own(catalog.author(jwt), id);
        mode.shareCode = Codes.shareCode();
        return catalog.detail(mode, true);
    }

    /** Withdrawn: no longer listed nor downloadable (its versions stay, for an administrator). */
    @DELETE
    @Path("/{id}")
    @Transactional
    public Views.ModeDetail withdraw(@PathParam("id") String id) {
        Mode mode = catalog.own(catalog.author(jwt), id);
        if (mode.withdrawnAt == null) {
            mode.withdrawnAt = Instant.now();
            mode.withdrawnReason = "withdrawn by its author";
        }
        return catalog.detail(mode, true);
    }

    private static Mode publicMode(String id) {
        return Mode.byPublicId(id).filter(Mode::isPublic).orElseThrow(() -> Problem.notFound("no such mode"));
    }

    private static InputStream open(FileUpload file) throws IOException {
        return file == null ? null : Files.newInputStream(file.uploadedFile());
    }
}
