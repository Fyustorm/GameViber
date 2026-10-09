package fr.fyustorm.gameviber.catalog;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Comparator;
import java.util.Optional;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

import org.eclipse.microprofile.config.inject.ConfigProperty;

import jakarta.enterprise.context.ApplicationScoped;

/** The published packages: `packages/<mode public id>/<version number>.gameviber` in the data directory. */
@ApplicationScoped
public class PackageStore {
    @ConfigProperty(name = "data.dir")
    String dataDir;

    public Path path(Mode mode, int version) {
        return Path.of(dataDir, "packages", mode.publicId, version + ".gameviber");
    }

    /** The first of its captures (by name): what shows the mode on the site and in link previews. */
    public Optional<byte[]> cover(Mode mode, int version) throws IOException {
        try (var zip = new ZipFile(path(mode, version).toFile())) {
            var first = zip.stream()
                    .filter(e -> e.getName().startsWith("captures/") && e.getName().endsWith(".png"))
                    .min(Comparator.comparing(ZipEntry::getName));
            if (first.isEmpty()) {
                return Optional.empty();
            }
            try (var in = zip.getInputStream(first.get())) {
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
