package fr.fyustorm.gameviber.catalog;

import java.io.IOException;
import java.net.URI;
import java.net.URISyntaxException;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.time.Duration;
import java.time.Instant;
import java.util.List;
import java.util.Objects;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;

import org.eclipse.microprofile.config.inject.ConfigProperty;
import org.jboss.logging.Logger;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;

import io.quarkus.narayana.jta.QuarkusTransaction;
import io.quarkus.runtime.Startup;
import jakarta.annotation.PostConstruct;
import jakarta.annotation.PreDestroy;
import jakarta.enterprise.context.ApplicationScoped;
import jakarta.inject.Inject;

/**
 * Games' banners: the header image of their Steam store page. Steam keeps the
 * images of recent games under a path with a hash only its store's API tells,
 * asked here in the background for the games with a Steam app id and kept with
 * them; the path without a hash (`legacy`), which older games still answer, is
 * shown while Steam was not asked or gave none.
 */
@Startup
@ApplicationScoped
public class SteamBanners {
    private static final Logger log = Logger.getLogger(SteamBanners.class);
    /** A banner kept is checked again this often: a store page may change it. */
    static final Duration CHECK_EVERY = Duration.ofDays(7);
    private static final Duration SWEEP_EVERY = Duration.ofHours(6);
    /** Steam's store API takes about 200 requests every 5 minutes. */
    private static final Duration PAUSE = Duration.ofMillis(1500);

    @ConfigProperty(name = "steam.banners", defaultValue = "true")
    boolean enabled;
    @Inject
    ObjectMapper json;

    private final HttpClient http = HttpClient.newBuilder()
            .connectTimeout(Duration.ofSeconds(10))
            .followRedirects(HttpClient.Redirect.NORMAL)
            .build();
    private ScheduledExecutorService worker;

    record Due(long game, long app) {}

    @PostConstruct
    void start() {
        if (!enabled) {
            return;
        }
        worker = Executors.newSingleThreadScheduledExecutor(task -> {
            Thread thread = new Thread(task, "steam-banners");
            thread.setDaemon(true);
            return thread;
        });
        worker.scheduleWithFixedDelay(this::sweep, 10, SWEEP_EVERY.toSeconds(), TimeUnit.SECONDS);
    }

    @PreDestroy
    void stop() {
        if (worker != null) {
            worker.shutdownNow();
        }
    }

    /** A game got a Steam app id: its banner is asked once the change is saved. */
    public void soon() {
        if (worker != null) {
            worker.schedule(this::sweep, 5, TimeUnit.SECONDS);
        }
    }

    /** The banner shown for `game`, none without a Steam app id. */
    public static String of(Game game) {
        if (game.steamAppId == null) {
            return null;
        }
        return game.bannerUrl != null ? game.bannerUrl : legacy(game.steamAppId);
    }

    static String legacy(long app) {
        return "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/" + app + "/header.jpg";
    }

    /** Asks Steam for the banners not asked yet, or not lately. */
    void sweep() {
        try {
            Instant before = Instant.now().minus(CHECK_EVERY);
            List<Due> due = QuarkusTransaction.requiringNew().call(() -> Game.<Game>list(
                    "steamAppId is not null and (bannerCheckedAt is null or bannerCheckedAt < ?1)", before)
                    .stream().map(g -> new Due(g.id, g.steamAppId)).toList());
            for (Due one : due) {
                String url;
                try {
                    url = header(one.app());
                } catch (IOException e) {
                    // Steam out of reach: asked again at the next sweep.
                    log.debugf("no banner for Steam app %d: %s", one.app(), e.getMessage());
                    continue;
                }
                QuarkusTransaction.requiringNew().run(() -> {
                    Game game = Game.findById(one.game());
                    // Its app id changed meanwhile: the next sweep asks for the new one.
                    if (game != null && Objects.equals(game.steamAppId, one.app())) {
                        game.bannerUrl = url;
                        game.bannerCheckedAt = Instant.now();
                    }
                });
                Thread.sleep(PAUSE.toMillis());
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        } catch (RuntimeException e) {
            // A failure must not stop the sweeps to come.
            log.warn("banner sweep failed", e);
        }
    }

    /** The header image of the Steam app `app`, null when its store page gives none. */
    String header(long app) throws IOException, InterruptedException {
        var request = HttpRequest.newBuilder(URI.create("https://store.steampowered.com/api/appdetails?filters=basic&appids=" + app))
                .timeout(Duration.ofSeconds(20))
                .header("User-Agent", "GameViber community server")
                .build();
        HttpResponse<String> response = http.send(request, HttpResponse.BodyHandlers.ofString());
        if (response.statusCode() != 200) {
            throw new IOException("Steam answered " + response.statusCode());
        }
        JsonNode answer = json.readTree(response.body());
        // Steam says "null" when asked too often.
        if (!answer.has(String.valueOf(app))) {
            throw new IOException("Steam gave no answer");
        }
        return headerOf(answer, app);
    }

    /** `header_image` in Steam's answer about `app`, if it is an image of Steam's. */
    static String headerOf(JsonNode answer, long app) {
        JsonNode entry = answer.path(String.valueOf(app));
        if (!entry.path("success").asBoolean(false)) {
            return null;
        }
        String url = entry.path("data").path("header_image").asText("");
        return trusted(url) ? url : null;
    }

    /** Shown on the website and in link previews: only Steam's own images. */
    static boolean trusted(String url) {
        try {
            URI uri = new URI(url);
            return "https".equals(uri.getScheme()) && uri.getHost() != null && uri.getHost().endsWith(".steamstatic.com");
        } catch (URISyntaxException e) {
            return false;
        }
    }
}
