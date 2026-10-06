package fr.fyustorm.gameviber.db;

import java.time.Instant;

import jakarta.persistence.AttributeConverter;
import jakarta.persistence.Converter;

/**
 * Moments as milliseconds since the epoch: SQLite has no date type, and its driver and
 * Hibernate do not agree on a text format for one.
 */
@Converter(autoApply = true)
public class EpochMillis implements AttributeConverter<Instant, Long> {
    @Override
    public Long convertToDatabaseColumn(Instant instant) {
        return instant == null ? null : instant.toEpochMilli();
    }

    @Override
    public Instant convertToEntityAttribute(Long millis) {
        return millis == null ? null : Instant.ofEpochMilli(millis);
    }
}
