package fr.fyustorm.gameviber.auth;

import java.time.Instant;
import java.util.Locale;
import java.util.Optional;

import io.quarkus.hibernate.orm.panache.PanacheEntityBase;
import jakarta.persistence.Column;
import jakarta.persistence.Entity;
import jakarta.persistence.GeneratedValue;
import jakarta.persistence.GenerationType;
import jakarta.persistence.Id;

/** Who publishes modes: a pseudo and a password, nothing else known about them. */
@Entity
public class Author extends PanacheEntityBase {
    @Id
    @GeneratedValue(strategy = GenerationType.IDENTITY)
    public Long id;
    public String pseudo;
    @Column(name = "pseudo_key")
    public String pseudoKey;
    @Column(name = "password_hash")
    public String passwordHash;
    public boolean blocked;
    @Column(name = "created_at")
    public Instant createdAt;

    public static String key(String pseudo) {
        return pseudo.trim().toLowerCase(Locale.ROOT);
    }

    public static Optional<Author> byPseudo(String pseudo) {
        return find("pseudoKey", key(pseudo)).firstResultOptional();
    }
}
