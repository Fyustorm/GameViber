package fr.fyustorm.gameviber.catalog;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.ConcurrentHashMap;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

import org.eclipse.microprofile.config.inject.ConfigProperty;

import com.fasterxml.jackson.databind.ObjectMapper;

import jakarta.enterprise.context.ApplicationScoped;

/** The published packages: `packages/<mode public id>/<version number>.gameviber` in the data directory. */
@ApplicationScoped
public class PackageStore {
    @ConfigProperty(name = "data.dir")
    String dataDir;
    /** What each version's mode is made for and sets up: a version's package never changes. */
    private final Map<Path, Described> described = new ConcurrentHashMap<>();
    private static final ObjectMapper JSON = new ObjectMapper();

    /** `SharedPackage.uses` and `ModeSetup` of a version. */
    public record Described(List<String> uses, ModeSetup setup) {}

    public Path path(Mode mode, int version) {
        return Path.of(dataDir, "packages", mode.publicId, version + ".gameviber");
    }

    /**
     * What shows the mode on the site, in the app and in link previews: the capture its
     * author chose (`cover` in the manifest's inputs), else the first of its captures (by name).
     */
    public Optional<byte[]> cover(Mode mode, int version) throws IOException {
        try (var zip = new ZipFile(path(mode, version).toFile())) {
            var chosen = Optional.ofNullable(zip.getEntry("gameviber.json")).flatMap(manifest -> {
                try (var in = zip.getInputStream(manifest)) {
                    String file = JSON.readTree(in).path("inputs").path("cover").asText("");
                    return file.isEmpty() || file.contains("/") ? Optional.empty() : Optional.ofNullable(zip.getEntry("captures/" + file));
                } catch (IOException e) {
                    return Optional.empty();
                }
            });
            var cover = chosen.or(() -> zip.stream()
                    .filter(e -> e.getName().startsWith("captures/") && e.getName().endsWith(".png"))
                    .min(Comparator.comparing(ZipEntry::getName)));
            if (cover.isEmpty()) {
                return Optional.empty();
            }
            try (var in = zip.getInputStream(cover.get())) {
                return Optional.of(in.readAllBytes());
            }
        }
    }

    /** What a version's mode is made for and sets up, read from its package once. */
    public Described describe(Mode mode, int version) {
        return described.computeIfAbsent(path(mode, version), path -> {
            Map<String, byte[]> entries = new LinkedHashMap<>();
            try (var zip = new ZipFile(path.toFile())) {
                for (var entry : zip.stream().toList()) {
                    String name = entry.getName();
                    boolean read = name.endsWith(".json") || name.endsWith(".luau");
                    try (var in = zip.getInputStream(entry)) {
                        entries.put(name, read ? in.readAllBytes() : new byte[0]);
                    }
                }
            } catch (IOException e) {
                return new Described(List.of(), ModeSetup.NONE);
            }
            return new Described(List.copyOf(SharedPackage.uses(entries)), ModeSetup.of(entries));
        });
    }

    /** One of a version's captures, by its file name. */
    public Optional<byte[]> capture(Mode mode, int version, String file) throws IOException {
        try (var zip = new ZipFile(path(mode, version).toFile())) {
            var entry = zip.getEntry("captures/" + file);
            if (entry == null || file.contains("/")) {
                return Optional.empty();
            }
            try (var in = zip.getInputStream(entry)) {
                return Optional.of(in.readAllBytes());
            }
        }
    }

    /** Written beside, then renamed: a failure leaves no half file. */
    public void save(Mode mode, int version, byte[] bytes) throws IOException {
        Path path = path(mode, version);
        Files.createDirectories(path.getParent());
        Path partial = path.resolveSibling(path.getFileName() + ".part");
        Files.write(partial, bytes);
        Files.move(partial, path, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
    }
}
