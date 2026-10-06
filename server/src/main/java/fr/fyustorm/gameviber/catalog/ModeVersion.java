package fr.fyustorm.gameviber.catalog;

import java.time.Instant;
import java.util.List;
import java.util.Optional;

import io.quarkus.hibernate.orm.panache.PanacheEntityBase;
import jakarta.persistence.Column;
import jakarta.persistence.Entity;
import jakarta.persistence.GeneratedValue;
import jakarta.persistence.GenerationType;
import jakarta.persistence.Id;
import jakarta.persistence.Table;

/** A version of a mode: its package, as published (`PackageStore`). */
@Entity
@Table(name = "mode_version")
public class ModeVersion extends PanacheEntityBase {
    @Id
    @GeneratedValue(strategy = GenerationType.IDENTITY)
    public Long id;
    @Column(name = "mode_id")
    public Long modeId;
    public int number;
    public String changelog;
    public int api;
    public long size;
    public String sha256;
    @Column(name = "created_at")
    public Instant createdAt;

    /** Newest first. */
    public static List<ModeVersion> of(Mode mode) {
        return list("modeId = ?1 order by number desc", mode.id);
    }

    public static Optional<ModeVersion> latest(Mode mode) {
        return find("modeId = ?1 order by number desc", mode.id).firstResultOptional();
    }
}
