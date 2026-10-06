package fr.fyustorm.gameviber.api;

import java.time.Duration;
import java.time.Instant;
import java.util.ArrayDeque;
import java.util.Deque;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

import io.vertx.core.http.HttpServerRequest;
import jakarta.enterprise.context.ApplicationScoped;
import jakarta.ws.rs.core.Response;

/**
 * Sliding-window limits per client address, in memory: enough for one
 * server, and forgotten on restart, which only ever lets a few more through.
 */
@ApplicationScoped
public class RateLimiter {
    private final Map<String, Deque<Instant>> windows = new ConcurrentHashMap<>();

    /** Counts one `action` of the client of `request`; refuses past `max` an hour. */
    public void check(String action, HttpServerRequest request, int max) {
        String key = action + "|" + address(request);
        Instant now = Instant.now();
        Instant since = now.minus(Duration.ofHours(1));
        Deque<Instant> window = windows.computeIfAbsent(key, k -> new ArrayDeque<>());
        synchronized (window) {
            while (!window.isEmpty() && window.peekFirst().isBefore(since)) {
                window.pollFirst();
            }
            if (window.size() >= max) {
                throw Problem.of(Response.Status.TOO_MANY_REQUESTS, "too many attempts: try again later");
            }
            window.addLast(now);
        }
    }

    private static String address(HttpServerRequest request) {
        var remote = request.remoteAddress();
        return remote == null ? "unknown" : remote.hostAddress();
    }
}
