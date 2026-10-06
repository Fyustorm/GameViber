package fr.fyustorm.gameviber.api;

import io.quarkus.runtime.annotations.RegisterForReflection;
import jakarta.ws.rs.WebApplicationException;
import jakarta.ws.rs.core.MediaType;
import jakarta.ws.rs.core.Response;

/** An error said to the client in a sentence it can show: {"error": "..."}. */
public final class Problem {
    /** Built in responses, not returned by an endpoint: registered for the native image's JSON. */
    @RegisterForReflection
    public record Body(String error) {}

    private Problem() {}

    public static WebApplicationException of(Response.Status status, String message) {
        return new WebApplicationException(message, Response.status(status).type(MediaType.APPLICATION_JSON).entity(new Body(message)).build());
    }

    public static WebApplicationException badRequest(String message) {
        return of(Response.Status.BAD_REQUEST, message);
    }

    public static WebApplicationException notFound(String message) {
        return of(Response.Status.NOT_FOUND, message);
    }

    public static WebApplicationException forbidden(String message) {
        return of(Response.Status.FORBIDDEN, message);
    }
}
