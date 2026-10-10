package fr.fyustorm.gameviber.catalog;

import java.io.IOException;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;

/**
 * What a mode's package sets up, shown on its page: the phases and indicators
 * the player set up for it (`mode.json` in the app, the `inputs` of
 * `gameviber.json` here; `gameviber/src/package.rs`, whose older names are
 * read too), its captures by phase, its variants and funscripts. The phases a
 * script declares itself are not read: only GameViber runs scripts.
 */
public record ModeSetup(
        List<Phase> phases, List<Indicator> indicators, List<Capture> captures, List<String> variants, List<String> funscripts) {

    /** `sound` and `screen`: how the phase sounds and looks, as the phase models are told. */
    public record Phase(String name, String sound, String screen, List<String> indicators) {}

    /** `kind`: gauge (how full a bar is) or visibility (whether an element shows). */
    public record Indicator(String name, String kind, List<Zone> zones) {}

    /**
     * Left, top, width, height as fractions of the screen, and the capture it
     * shows on: the one it was drawn on, else the first of its phase.
     */
    public record Zone(List<Double> rect, String capture) {}

    public record Capture(String file, String phase) {}

    public static final ModeSetup NONE = new ModeSetup(List.of(), List.of(), List.of(), List.of(), List.of());
    private static final ObjectMapper JSON = new ObjectMapper();

    /** From a package's entries (the captures' and funscripts' names are enough). */
    static ModeSetup of(Map<String, byte[]> entries) {
        JsonNode inputs = JSON.createObjectNode();
        try {
            byte[] manifest = entries.get("gameviber.json");
            if (manifest != null) {
                inputs = JSON.readTree(manifest).path("inputs");
            }
        } catch (IOException e) {
            // Checked when published: unreadable, it sets nothing up.
        }
        List<Capture> captures = new ArrayList<>();
        for (JsonNode capture : inputs.path("captures")) {
            String file = capture.path("file").asText("");
            if (entries.containsKey("captures/" + file)) {
                captures.add(new Capture(file, text(capture, "phase", "scene")));
            }
        }
        List<Phase> phases = new ArrayList<>();
        for (JsonNode phase : either(inputs, "phases", "scenes")) {
            String name = phase.path("name").asText("");
            if (!name.isEmpty()) {
                phases.add(new Phase(name, text(phase, "sound"), text(phase, "screen"), names(phase, "indicators", "indicator", "zone")));
            }
        }
        // Zones naming the same indicator are one indicator.
        Map<String, Indicator> indicators = new LinkedHashMap<>();
        for (JsonNode zone : inputs.path("zones")) {
            String name = text(zone, "indicator", "name");
            if (name == null) {
                continue;
            }
            String kind = switch (zone.path("kind").asText("")) {
                case "gauge", "bar" -> "gauge";
                default -> "visibility";
            };
            List<Double> rect = new ArrayList<>();
            zone.path("rect").forEach(v -> rect.add(v.asDouble()));
            if (rect.size() != 4) {
                continue;
            }
            String own = text(zone, "capture");
            String phase = text(zone, "phase", "scene");
            String capture = captures.stream().map(Capture::file).filter(f -> f.equals(own)).findFirst()
                    .orElseGet(() -> captures.stream().filter(c -> c.phase() != null && c.phase().equals(phase)).map(Capture::file).findFirst().orElse(null));
            indicators.computeIfAbsent(name, n -> new Indicator(n, kind, new ArrayList<>())).zones().add(new Zone(rect, capture));
        }
        List<String> variants = files(entries, "variants/", ".luau");
        List<String> funscripts = files(entries, "funscripts/", ".funscript");
        return new ModeSetup(phases, List.copyOf(indicators.values()), captures, variants, funscripts);
    }

    /** The first of these keys that is there (the current name, then older ones). */
    private static JsonNode either(JsonNode node, String... keys) {
        for (String key : keys) {
            if (node.has(key)) {
                return node.get(key);
            }
        }
        return JSON.createArrayNode();
    }

    private static String text(JsonNode node, String... keys) {
        JsonNode value = either(node, keys);
        return value.isTextual() && !value.asText().isBlank() ? value.asText() : null;
    }

    /** One name or several. */
    private static List<String> names(JsonNode node, String... keys) {
        JsonNode value = either(node, keys);
        List<String> names = new ArrayList<>();
        if (value.isTextual()) {
            names.add(value.asText());
        }
        value.forEach(v -> names.add(v.asText()));
        return names;
    }

    private static List<String> files(Map<String, byte[]> entries, String dir, String extension) {
        return entries.keySet().stream()
                .filter(n -> n.startsWith(dir) && n.endsWith(extension))
                .map(n -> n.substring(dir.length(), n.length() - extension.length()))
                .sorted()
                .toList();
    }
}
