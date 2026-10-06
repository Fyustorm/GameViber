package fr.fyustorm.gameviber.catalog;

import java.time.Instant;
import java.util.Locale;
import java.util.Optional;

import io.quarkus.hibernate.orm.panache.PanacheEntityBase;
import jakarta.persistence.Column;
import jakarta.persistence.Entity;
import jakarta.persistence.GeneratedValue;
import jakarta.persistence.GenerationType;
import jakarta.persistence.Id;

/** A game modes are shared for, as the first mode published for it named it. */
@Entity
public class Game extends PanacheEntityBase {
    // SQLite numbers rows itself: no sequence.
    @Id
    @GeneratedValue(strategy = GenerationType.IDENTITY)
    public Long id;
    public String name;
    public String slug;
    @Column(name = "steam_app_id")
    public Long steamAppId;
    @Column(name = "created_at")
    public Instant createdAt;

    /** "ELDEN RING" and "Elden Ring: Nightreign" give "eldenring", "eldenringnightreign". */
    public static String slug(String name) {
        return name.toLowerCase(Locale.ROOT).replaceAll("[^\\p{L}\\p{N}]", "");
    }

    /** The game with this Steam app id, else of this name; created when there is none. */
    public static Game findOrCreate(String name, Long steamAppId) {
        Optional<Game> byApp = steamAppId == null ? Optional.empty() : find("steamAppId", steamAppId).firstResultOptional();
        Game game = byApp.or(() -> find("slug", slug(name)).firstResultOptional()).orElse(null);
        if (game == null) {
            game = new Game();
            game.name = name.trim();
            game.slug = slug(name);
            game.createdAt = Instant.now();
        }
        if (game.steamAppId == null && steamAppId != null) {
            game.steamAppId = steamAppId;
        }
        game.persist();
        return game;
    }
}
