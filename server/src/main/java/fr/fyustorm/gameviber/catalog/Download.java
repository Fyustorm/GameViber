package fr.fyustorm.gameviber.catalog;

import java.nio.file.Path;
import java.time.Instant;

import io.quarkus.hibernate.orm.panache.PanacheEntityBase;
import jakarta.persistence.Column;
import jakarta.persistence.Entity;
import jakarta.persistence.GeneratedValue;
import jakarta.persistence.GenerationType;
import jakarta.persistence.Id;

/** A package downloaded: when, and which version (nothing about who). */
@Entity
public class Download extends PanacheEntityBase {
    @Id
    @GeneratedValue(strategy = GenerationType.IDENTITY)
    public Long id;
    @Column(name = "mode_id")
    public Long modeId;
    @Column(name = "version_number")
    public int versionNumber;
    @Column(name = "created_at")
    public Instant createdAt;

    /** A package to send, and the file name to save it under. */
    public record Served(Path path, String fileName) {}
}
