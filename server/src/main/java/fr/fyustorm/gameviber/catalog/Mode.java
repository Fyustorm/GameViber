package fr.fyustorm.gameviber.catalog;

import java.time.Instant;
import java.util.Optional;

import io.quarkus.hibernate.orm.panache.PanacheEntityBase;
import jakarta.persistence.Column;
import jakarta.persistence.Entity;
import jakarta.persistence.GeneratedValue;
import jakarta.persistence.GenerationType;
import jakarta.persistence.Id;

/** A mode shared by its author, with its versions (`ModeVersion`). */
@Entity
public class Mode extends PanacheEntityBase {
    public static final String PRIVATE = "private";
    public static final String PUBLIC = "public";

    @Id
    @GeneratedValue(strategy = GenerationType.IDENTITY)
    public Long id;
    @Column(name = "public_id")
    public String publicId;
    @Column(name = "game_id")
    public Long gameId;
    @Column(name = "author_id")
    public Long authorId;
    public String name;
    public String description;
    public String visibility;
    @Column(name = "share_code")
    public String shareCode;
    public long downloads;
    @Column(name = "created_at")
    public Instant createdAt;
    @Column(name = "updated_at")
    public Instant updatedAt;
    @Column(name = "withdrawn_at")
    public Instant withdrawnAt;
    @Column(name = "withdrawn_reason")
    public String withdrawnReason;

    public boolean isPublic() {
        return PUBLIC.equals(visibility) && withdrawnAt == null;
    }

    public static Optional<Mode> byPublicId(String publicId) {
        return find("publicId", publicId).firstResultOptional();
    }

    public static Optional<Mode> byShareCode(String code) {
        return find("shareCode", code.trim().toUpperCase()).firstResultOptional();
    }
}
