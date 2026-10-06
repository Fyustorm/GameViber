package fr.fyustorm.gameviber.catalog;

import java.security.SecureRandom;

/** Random names: a mode's public id, the code that shares a private mode. */
final class Codes {
    /** No 0/O, 1/I/L: read aloud or typed from a screenshot. */
    private static final String ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";

    private Codes() {}

    private static String random(int length) {
        // Made here: a static one would be built into the native image with its seed.
        var random = new SecureRandom();
        var out = new StringBuilder(length);
        for (int i = 0; i < length; i++) {
            out.append(ALPHABET.charAt(random.nextInt(ALPHABET.length())));
        }
        return out.toString();
    }

    /** "k3x9m2q7ab": 10 characters, about 49 bits. */
    static String publicId() {
        return random(10).toLowerCase();
    }

    /** "GV-7KQ2-M9XD": 8 characters, about 39 bits, with sign-in and download limits. */
    static String shareCode() {
        String code = random(8);
        return "GV-" + code.substring(0, 4) + "-" + code.substring(4);
    }
}
