package fr.fyustorm.gameviber.catalog;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import java.util.zip.CRC32;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;
import java.util.zip.ZipOutputStream;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;

/**
 * A `.gameviber` file sent to be published, checked the way GameViber checks
 * one it imports (`gameviber/src/sharing.rs`), and cleaned: only the entries a
 * mode's package has, the captures stripped of everything but their image
 * (PNG text and metadata chunks can hold anything).
 */
public final class SharedPackage {
    /** The layout GameViber writes, and the mode API versions it runs. */
    static final int FORMAT = 2;
    static final Set<Integer> APIS = Set.of(1);
    static final int MAX_VARIANTS = 16;
    static final int MAX_CAPTURES = 8 * 40;
    static final int MAX_ENTRIES = 2 + MAX_VARIANTS + MAX_CAPTURES;
    static final long MAX_ENTRY_SIZE = 8L << 20;
    static final long MAX_TOTAL_SIZE = 64L << 20;
    static final int MAX_NAME_CHARS = 100;

    private static final String MANIFEST = "gameviber.json";
    private static final String SCRIPT = "mode.luau";
    private static final String VARIANTS = "variants/";
    private static final String CAPTURES = "captures/";
    private static final byte[] PNG_SIGNATURE = { (byte) 0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n' };
    /** PNG chunks kept: the image, its palette and transparency. */
    private static final Set<String> PNG_KEPT = Set.of("IHDR", "PLTE", "tRNS", "IDAT", "IEND");
    private static final Pattern API = Pattern.compile("\\bapi\\s*=\\s*(\\d+)");
    private static final Pattern SAFE_NAME = Pattern.compile("[A-Za-z0-9_\\-.]{1,128}");
    private static final ObjectMapper JSON = new ObjectMapper();

    /** The game's name and Steam app id, as the package names them. */
    public final String gameName;
    public final Long steamAppId;
    /** The mode API version its script declares. */
    public final int api;
    /** The package as stored and served. */
    public final byte[] bytes;
    public final String sha256;

    private SharedPackage(String gameName, Long steamAppId, int api, byte[] bytes) {
        this.gameName = gameName;
        this.steamAppId = steamAppId;
        this.api = api;
        this.bytes = bytes;
        this.sha256 = sha256(bytes);
    }

    /** Why a package is refused, said to whoever sent it. */
    public static final class Invalid extends Exception {
        public Invalid(String message) {
            super(message);
        }
    }

    public static SharedPackage check(InputStream upload) throws Invalid, IOException {
        Map<String, byte[]> entries = read(upload);
        byte[] manifestBytes = entries.get(MANIFEST);
        byte[] script = entries.get(SCRIPT);
        if (manifestBytes == null || script == null) {
            throw new Invalid("this is not a mode shared from GameViber: " + MANIFEST + " or " + SCRIPT + " is missing");
        }
        JsonNode manifest;
        try {
            manifest = JSON.readTree(manifestBytes);
        } catch (IOException e) {
            throw new Invalid(MANIFEST + " is not JSON");
        }
        int format = manifest.path("format").asInt(0);
        if (format != FORMAT) {
            throw new Invalid("this file was written by another GameViber (format " + format + "): export it again with the current one");
        }
        String gameName = manifest.path("game").path("name").asText("").trim();
        if (gameName.isEmpty() || gameName.length() > MAX_NAME_CHARS || Game.slug(gameName).isEmpty()) {
            throw new Invalid("this file names no game");
        }
        JsonNode app = manifest.path("game").path("steam_app_id");
        Long steamAppId = app.canConvertToLong() && app.asLong() > 0 ? app.asLong() : null;

        Map<String, byte[]> kept = new LinkedHashMap<>();
        kept.put(MANIFEST, manifestBytes);
        kept.put(SCRIPT, script);
        int api = api(SCRIPT, script);
        int variants = 0;
        for (var entry : entries.entrySet()) {
            String name = entry.getKey();
            if (name.equals(MANIFEST) || name.equals(SCRIPT)) {
                continue;
            }
            if (name.startsWith(VARIANTS) && name.endsWith(".luau") && safe(name.substring(VARIANTS.length()))) {
                if (++variants > MAX_VARIANTS) {
                    throw new Invalid("a mode has at most " + MAX_VARIANTS + " variants");
                }
                api(name, entry.getValue());
                kept.put(name, entry.getValue());
            } else if (name.startsWith(CAPTURES) && name.endsWith(".png") && safe(name.substring(CAPTURES.length()))) {
                kept.put(name, stripPng(name, entry.getValue()));
            } else {
                throw new Invalid("unexpected file in a mode: " + name);
            }
        }
        return new SharedPackage(gameName, steamAppId, api, write(kept));
    }

    /** The entries, within the limits (the sizes an archive states may lie: what is read counts). */
    private static Map<String, byte[]> read(InputStream upload) throws Invalid, IOException {
        Map<String, byte[]> entries = new LinkedHashMap<>();
        long total = 0;
        try (var zip = new ZipInputStream(upload)) {
            ZipEntry entry;
            while ((entry = zip.getNextEntry()) != null) {
                if (entry.isDirectory()) {
                    continue;
                }
                if (entries.size() >= MAX_ENTRIES) {
                    throw new Invalid("this file holds too many files to be a mode");
                }
                long limit = Math.min(MAX_ENTRY_SIZE, MAX_TOTAL_SIZE - total);
                byte[] bytes = zip.readNBytes((int) Math.min(limit + 1, Integer.MAX_VALUE));
                if (bytes.length > limit) {
                    throw new Invalid(entry.getName() + " is too large");
                }
                total += bytes.length;
                if (entries.put(entry.getName(), bytes) != null) {
                    throw new Invalid(entry.getName() + " is there twice");
                }
            }
        } catch (java.util.zip.ZipException e) {
            throw new Invalid("this is not a file of a shared mode");
        }
        if (entries.isEmpty()) {
            throw new Invalid("this is not a file of a shared mode");
        }
        return entries;
    }

    /** A plain file name: no directory, nothing hidden. */
    private static boolean safe(String name) {
        return SAFE_NAME.matcher(name).matches() && !name.startsWith(".");
    }

    /** The API version a script declares; it must be text, and one this GameViber runs. */
    private static int api(String name, byte[] script) throws Invalid {
        String text;
        try {
            text = StandardCharsets.UTF_8.newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(ByteBuffer.wrap(script))
                    .toString();
        } catch (CharacterCodingException e) {
            throw new Invalid(name + " is not text");
        }
        Matcher m = API.matcher(text);
        if (!m.find()) {
            throw new Invalid(name + " declares no mode API version (api = 1)");
        }
        int api = Integer.parseInt(m.group(1));
        if (!APIS.contains(api)) {
            throw new Invalid(name + " is written for the mode API " + api + ", which GameViber does not run yet");
        }
        return api;
    }

    /** A PNG with only its image chunks, each checked. */
    static byte[] stripPng(String name, byte[] png) throws Invalid {
        var in = ByteBuffer.wrap(png);
        if (png.length < PNG_SIGNATURE.length || !java.util.Arrays.equals(png, 0, PNG_SIGNATURE.length, PNG_SIGNATURE, 0, PNG_SIGNATURE.length)) {
            throw new Invalid(name + " is not a PNG image");
        }
        in.position(PNG_SIGNATURE.length);
        var out = new ByteArrayOutputStream(png.length);
        out.writeBytes(PNG_SIGNATURE);
        boolean ended = false;
        while (in.remaining() >= 12 && !ended) {
            int length = in.getInt();
            byte[] type = new byte[4];
            in.get(type);
            if (length < 0 || length > in.remaining() - 4) {
                throw new Invalid(name + " is a damaged PNG image");
            }
            byte[] data = new byte[length];
            in.get(data);
            int crc = in.getInt();
            var check = new CRC32();
            check.update(type);
            check.update(data);
            if ((int) check.getValue() != crc) {
                throw new Invalid(name + " is a damaged PNG image");
            }
            String chunk = new String(type, StandardCharsets.US_ASCII);
            if (PNG_KEPT.contains(chunk)) {
                out.writeBytes(ByteBuffer.allocate(4).putInt(length).array());
                out.writeBytes(type);
                out.writeBytes(data);
                out.writeBytes(ByteBuffer.allocate(4).putInt(crc).array());
            }
            ended = chunk.equals("IEND");
        }
        if (!ended) {
            throw new Invalid(name + " is a damaged PNG image");
        }
        return out.toByteArray();
    }

    private static byte[] write(Map<String, byte[]> entries) throws IOException {
        var out = new ByteArrayOutputStream();
        try (var zip = new ZipOutputStream(out)) {
            for (var entry : entries.entrySet()) {
                zip.putNextEntry(new ZipEntry(entry.getKey()));
                zip.write(entry.getValue());
                zip.closeEntry();
            }
        }
        return out.toByteArray();
    }

    static String sha256(byte[] bytes) {
        try {
            return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
    }

    /** Reads it back (tests). */
    static Map<String, byte[]> entries(byte[] bytes) throws IOException, Invalid {
        return read(new ByteArrayInputStream(bytes));
    }
}
