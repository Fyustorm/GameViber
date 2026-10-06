package fr.fyustorm.gameviber.db;

import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.file.Files;
import java.nio.file.Path;

import org.eclipse.microprofile.config.inject.ConfigProperty;

import io.quarkus.liquibase.LiquibaseFactory;
import io.quarkus.runtime.Startup;
import jakarta.annotation.PostConstruct;
import jakarta.enterprise.context.ApplicationScoped;
import jakarta.inject.Inject;
import liquibase.Liquibase;

/**
 * Brings the database up to date at start, once its directory exists: SQLite
 * does not create it, and Liquibase's own migration at start runs too early to
 * make it first.
 */
@Startup
@ApplicationScoped
public class Migrations {
    @ConfigProperty(name = "data.dir")
    String dataDir;
    @Inject
    LiquibaseFactory liquibase;

    @PostConstruct
    void migrate() {
        try {
            Files.createDirectories(Path.of(dataDir));
            try (Liquibase migrations = liquibase.createLiquibase()) {
                migrations.update(liquibase.createContexts(), liquibase.createLabels());
            }
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        } catch (Exception e) {
            throw new IllegalStateException("cannot bring the database up to date", e);
        }
    }
}
