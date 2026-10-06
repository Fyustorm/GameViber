package fr.fyustorm.gameviber.catalog;

import java.time.Instant;
import java.util.Set;

import io.quarkus.hibernate.orm.panache.PanacheEntityBase;
import jakarta.persistence.Column;
import jakarta.persistence.Entity;
import jakarta.persistence.GeneratedValue;
import jakarta.persistence.GenerationType;
import jakarta.persistence.Id;

/** A player telling a mode is broken by a game update, or should not be there. */
@Entity
public class Report extends PanacheEntityBase {
    public static final Set<String> REASONS = Set.of("broken", "content", "other");

    @Id
    @GeneratedValue(strategy = GenerationType.IDENTITY)
    public Long id;
    @Column(name = "mode_id")
    public Long modeId;
    public String reason;
    public String details;
    @Column(name = "created_at")
    public Instant createdAt;
    @Column(name = "resolved_at")
    public Instant resolvedAt;
}
