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
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
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
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;

/**
 * A `.gameviber` file sent to be published, checked the way GameViber checks
 * one it imports (`gameviber/src/sharing.rs`), and cleaned: only the entries a
 * mode's package has, the captures stripped of everything but their image
 * (PNG text and metadata chunks can hold anything), the funscripts of
 * everything but their actions.
 */
public final class SharedPackage {
    /** The layout GameViber writes, and the mode API versions it runs. */
    static final int FORMAT = 2;
    static final Set<Integer> APIS = Set.of(1);
    static final int MAX_VARIANTS = 16;
    static final int MAX_CAPTURES = 8 * 40;
    /** Funscripts in a package, and points in one (`gameviber/src/funscript.rs`). */
    static final int MAX_FUNSCRIPTS = 32;
    static final int MAX_FUNSCRIPT_POINTS = 20_000;
    static final int MAX_ENTRIES = 2 + MAX_VARIANTS + MAX_CAPTURES + MAX_FUNSCRIPTS;
    static final long MAX_ENTRY_SIZE = 8L << 20;
    static final long MAX_TOTAL_SIZE = 64L << 20;
    static final int MAX_NAME_CHARS = 100;

    private static final String MANIFEST = "gameviber.json";
    private static final String SCRIPT = "mode.luau";
    private static final String VARIANTS = "variants/";
    private static final String CAPTURES = "captures/";
    private static final String FUNSCRIPTS = "funscripts/";
    private static final byte[] PNG_SIGNATURE = { (byte) 0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n' };
    /** PNG chunks kept: the image, its palette and transparency. */
    private static final Set<String> PNG_KEPT = Set.of("IHDR", "PLTE", "tRNS", "IDAT", "IEND");
    private static final Pattern API = Pattern.compile("\\bapi\\s*=\\s*(\\d+)");
    private static final Pattern SAFE_NAME = Pattern.compile("[A-Za-z0-9_\\-.]{1,128}");
    private static final ObjectMapper JSON = new ObjectMapper();

    /** What a mode is made for, beyond the gamepad and the sound every mode reads (`uses`). */
    public static final String STROKERS = "strokers";
    public static final String SCREEN = "screen";
    public static final String PROGRAMS = "programs";
    /** The mode API's calls and fields (`docs/spec-modes.md`), the first names included. */
    private static final Pattern STROKES = Pattern.compile("\\b(?:stroke|thrust|funscript)\\s*\\(|\\bmotion\\s*[({]");
    private static final Pattern READS_SCREEN = Pattern.compile("\\binput\\.(?:screen|indicators|zones)\\b");
    private static final Pattern READS_PROGRAMS = Pattern.compile("\\binput\\.(?:external|custom)\\b");
    private static final Pattern LUA_COMMENT = Pattern.compile("--\\[(=*)\\[.*?\\]\\1\\]|--[^\\n]*", Pattern.DOTALL);

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
        int funscripts = 0;
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
            } else if (name.startsWith(FUNSCRIPTS) && name.endsWith(".funscript") && safe(name.substring(FUNSCRIPTS.length()))) {
                if (++funscripts > MAX_FUNSCRIPTS) {
                    throw new Invalid("a mode has at most " + MAX_FUNSCRIPTS + " funscripts");
                }
                kept.put(name, cleanFunscript(name, entry.getValue()));
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
    /** A funscript's actions (and whether it is inverted), nothing else. */
    static byte[] cleanFunscript(String name, byte[] funscript) throws Invalid {
        JsonNode actions;
        boolean inverted;
        try {
            JsonNode root = JSON.readTree(funscript);
            actions = root.path("actions");
            inverted = root.path("inverted").asBoolean(false);
        } catch (IOException e) {
            throw new Invalid(name + " is not JSON");
        }
        if (!actions.isArray() || actions.size() < 2 || actions.size() > MAX_FUNSCRIPT_POINTS) {
            throw new Invalid(name + " is not a funscript of 2 to " + MAX_FUNSCRIPT_POINTS + " actions");
        }
        ObjectNode clean = JSON.createObjectNode();
        clean.put("inverted", inverted);
        ArrayNode kept = clean.putArray("actions");
        for (JsonNode action : actions) {
            JsonNode at = action.path("at");
            JsonNode pos = action.path("pos");
            if (!at.isNumber() || !pos.isNumber() || at.asDouble() < 0 || !Double.isFinite(at.asDouble()) || !Double.isFinite(pos.asDouble())) {
                throw new Invalid(name + " has an action without a time and a position");
            }
            kept.addObject().put("at", at.asDouble()).put("pos", pos.asDouble());
        }
        try {
            return JSON.writeValueAsBytes(clean);
        } catch (IOException e) {
            throw new Invalid(name + " cannot be written again");
        }
    }

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

    /**
     * What the mode of a package's entries is made for: {@link #STROKERS} (its
     * scripts give strokes or motions, or it has funscripts: every mode plays on
     * every toy, this one was written for strokers too), {@link #SCREEN} (it
     * reads indicators or the game's image), {@link #PROGRAMS} (it reads values
     * other programs send).
     */
    static List<String> uses(Map<String, byte[]> entries) {
        StringBuilder scripts = new StringBuilder();
        boolean funscripts = false;
        for (var entry : entries.entrySet()) {
            String name = entry.getKey();
            if (name.equals(SCRIPT) || name.startsWith(VARIANTS) && name.endsWith(".luau")) {
                scripts.append(LUA_COMMENT.matcher(new String(entry.getValue(), StandardCharsets.UTF_8)).replaceAll("")).append('\n');
            }
            funscripts |= name.startsWith(FUNSCRIPTS) && name.endsWith(".funscript");
        }
        JsonNode inputs = JSON.createObjectNode();
        try {
            byte[] manifest = entries.get(MANIFEST);
            if (manifest != null) {
                inputs = JSON.readTree(manifest).path("inputs");
            }
        } catch (IOException e) {
            // Checked when published: unreadable, it sets nothing up.
        }
        List<String> uses = new ArrayList<>();
        if (funscripts || STROKES.matcher(scripts).find()) {
            uses.add(STROKERS);
        }
        if (inputs.path("zones").size() > 0 || READS_SCREEN.matcher(scripts).find()) {
            uses.add(SCREEN);
        }
        if (inputs.path("external").size() > 0 || inputs.path("inputs").size() > 0 || READS_PROGRAMS.matcher(scripts).find()) {
            uses.add(PROGRAMS);
        }
        return uses;
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
