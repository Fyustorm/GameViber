package fr.fyustorm.gameviber.stats;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.time.Instant;
import java.time.LocalDate;
import java.time.ZoneOffset;
import java.util.HexFormat;
import java.util.List;

import org.eclipse.microprofile.config.inject.ConfigProperty;

import io.quarkus.runtime.annotations.RegisterForReflection;
import jakarta.enterprise.context.ApplicationScoped;
import jakarta.inject.Inject;
import jakarta.persistence.EntityManager;

/**
 * Play time and votes, and the figures modes are ranked by: who kept playing
 * a mode says more of it than who downloaded it.
 */
@ApplicationScoped
public class Stats {
    /** A report counts this much at most: more is a broken or lying client. */
    static final long MAX_SECONDS = 24 * 3600;
    static final int MAX_SESSIONS = 50;
    /** Days a play counts half as much in the trend. */
    static final double TREND_HALF_LIFE_DAYS = 7;
    static final int TREND_DAYS = 28;

    @Inject
    EntityManager db;
    @ConfigProperty(name = "stats.salt")
    String salt;

    /** What the figures of a mode are, for players (in responses: registered for the native image's JSON). */
    @RegisterForReflection
    public record Figures(
            /** Installations that played it in the last 30 days. */
            long players,
            /** Median play time per installation, in minutes. */
            long medianMinutes,
            /** Share of its players who played it 3 sessions or more, 0..1. */
            double cameBack,
            long likes,
            long dislikes,
            /** Lower bound of the share who like it (Wilson, 95 %): ranks "Top rated". */
            double rating,
            /** Recent sessions, the latest counting more: ranks "Trending". */
            double trend) {}

    /** The installation id as stored: hashed with the server's salt. */
    public String installation(String id) {
        try {
            byte[] hash = MessageDigest.getInstance("SHA-256").digest((salt + ":" + id).getBytes(StandardCharsets.UTF_8));
            return HexFormat.of().formatHex(hash);
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
    }

    static long today() {
        return LocalDate.now(ZoneOffset.UTC).toEpochDay();
    }

    /** Adds play time of `installation` (hashed) with the mode `modeId`, today. */
    public void play(long modeId, String installation, long seconds, int sessions) {
        db.createNativeQuery("""
                INSERT INTO play (mode_id, installation, day, seconds, sessions) VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT (mode_id, installation, day) DO UPDATE SET
                    seconds = min(seconds + excluded.seconds, ?6),
                    sessions = min(sessions + excluded.sessions, ?7)
                """)
                .setParameter(1, modeId).setParameter(2, installation).setParameter(3, today())
                .setParameter(4, Math.clamp(seconds, 0, MAX_SECONDS)).setParameter(5, Math.clamp(sessions, 0, MAX_SESSIONS))
                .setParameter(6, MAX_SECONDS).setParameter(7, MAX_SESSIONS)
                .executeUpdate();
    }

    /** `installation` played the mode: only then may it vote for it. */
    public boolean played(long modeId, String installation) {
        Number n = (Number) db.createNativeQuery("SELECT count(*) FROM play WHERE mode_id = ?1 AND installation = ?2")
                .setParameter(1, modeId).setParameter(2, installation).getSingleResult();
        return n.longValue() > 0;
    }

    /** 1, -1, or 0 to take the vote back. */
    public void vote(long modeId, String installation, int value) {
        if (value == 0) {
            db.createNativeQuery("DELETE FROM vote WHERE mode_id = ?1 AND installation = ?2").setParameter(1, modeId).setParameter(2, installation).executeUpdate();
            return;
        }
        db.createNativeQuery("""
                INSERT INTO vote (mode_id, installation, value, updated_at) VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT (mode_id, installation) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
                """)
                .setParameter(1, modeId).setParameter(2, installation).setParameter(3, value).setParameter(4, Instant.now().toEpochMilli())
                .executeUpdate();
    }

    @SuppressWarnings("unchecked")
    public Figures figures(long modeId) {
        long today = today();
        Number players = (Number) db.createNativeQuery("SELECT count(DISTINCT installation) FROM play WHERE mode_id = ?1 AND day > ?2")
                .setParameter(1, modeId).setParameter(2, today - 30).getSingleResult();
        // Per installation: its play time and sessions, all time.
        List<Object[]> totals = db.createNativeQuery("SELECT sum(seconds), sum(sessions) FROM play WHERE mode_id = ?1 GROUP BY installation ORDER BY sum(seconds)")
                .setParameter(1, modeId).getResultList();
        long median = totals.isEmpty() ? 0 : ((Number) totals.get(totals.size() / 2)[0]).longValue() / 60;
        long cameBack = totals.stream().filter(t -> ((Number) t[1]).longValue() >= 3).count();
        List<Object[]> votes = db.createNativeQuery("SELECT value, count(*) FROM vote WHERE mode_id = ?1 GROUP BY value").setParameter(1, modeId).getResultList();
        long likes = 0, dislikes = 0;
        for (Object[] v : votes) {
            if (((Number) v[0]).intValue() > 0) {
                likes = ((Number) v[1]).longValue();
            } else {
                dislikes = ((Number) v[1]).longValue();
            }
        }
        List<Object[]> recent = db.createNativeQuery("SELECT day, sum(sessions) FROM play WHERE mode_id = ?1 AND day > ?2 GROUP BY day")
                .setParameter(1, modeId).setParameter(2, today - TREND_DAYS).getResultList();
        double trend = 0;
        for (Object[] r : recent) {
            double age = today - ((Number) r[0]).longValue();
            trend += ((Number) r[1]).doubleValue() * Math.pow(0.5, age / TREND_HALF_LIFE_DAYS);
        }
        return new Figures(
                players.longValue(), median, totals.isEmpty() ? 0 : (double) cameBack / totals.size(),
                likes, dislikes, wilson(likes, likes + dislikes), trend);
    }

    /** The lower bound of the share of likes, at 95 %: few votes count for little. */
    static double wilson(long likes, long votes) {
        if (votes == 0) {
            return 0;
        }
        double z = 1.96, n = votes, p = likes / n;
        return (p + z * z / (2 * n) - z * Math.sqrt((p * (1 - p) + z * z / (4 * n)) / n)) / (1 + z * z / n);
    }
}
