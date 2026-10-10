package fr.fyustorm.gameviber.catalog;

import java.time.Instant;
import java.util.List;

import fr.fyustorm.gameviber.auth.Author;
import fr.fyustorm.gameviber.stats.Stats.Figures;

/** What the API answers with. */
public final class Views {
    private Views() {}

    public record GameView(long id, String name, Long steamAppId, long modes) {}

    public record VersionView(int number, String changelog, int api, long size, String sha256, Instant createdAt) {
        static VersionView of(ModeVersion v) {
            return new VersionView(v.number, v.changelog, v.api, v.size, v.sha256, v.createdAt);
        }
    }

    /**
     * `version` and `api`: its latest version's number and mode API; `uses`:
     * what that version is made for (`SharedPackage.uses`).
     */
    public record ModeSummary(
            String id,
            String name,
            String description,
            String author,
            long downloads,
            int version,
            int api,
            List<String> uses,
            Instant updatedAt,
            Figures figures) {}

    /** `visibility` and `shareCode` only for its author (and administrators). */
    public record ModeDetail(
            String id,
            String name,
            String description,
            GameView game,
            String author,
            long downloads,
            List<VersionView> versions,
            List<String> uses,
            ModeSetup setup,
            Instant createdAt,
            Instant updatedAt,
            String visibility,
            String shareCode,
            Instant withdrawnAt,
            String withdrawnReason,
            Figures figures) {}

    public static GameView game(Game game) {
        long modes = Mode.count("gameId = ?1 and visibility = ?2 and withdrawnAt is null", game.id, Mode.PUBLIC);
        return new GameView(game.id, game.name, game.steamAppId, modes);
    }

    static ModeSummary summary(Mode mode, Figures figures, PackageStore store) {
        Author author = Author.findById(mode.authorId);
        var latest = ModeVersion.latest(mode);
        return new ModeSummary(
                mode.publicId, mode.name, mode.description, author.pseudo, mode.downloads,
                latest.map(v -> v.number).orElse(0), latest.map(v -> v.api).orElse(0),
                latest.map(v -> store.describe(mode, v.number).uses()).orElse(List.of()), mode.updatedAt, figures);
    }

    static ModeDetail detail(Mode mode, boolean owner, Figures figures, PackageStore store) {
        Author author = Author.findById(mode.authorId);
        Game game = Game.findById(mode.gameId);
        List<ModeVersion> all = ModeVersion.of(mode);
        List<VersionView> versions = all.stream().map(VersionView::of).toList();
        var described = all.isEmpty() ? new PackageStore.Described(List.of(), ModeSetup.NONE) : store.describe(mode, all.get(0).number);
        return new ModeDetail(
                mode.publicId,
                mode.name,
                mode.description,
                game(game),
                author.pseudo,
                mode.downloads,
                versions,
                described.uses(),
                described.setup(),
                mode.createdAt,
                mode.updatedAt,
                owner ? mode.visibility : null,
                owner ? mode.shareCode : null,
                mode.withdrawnAt,
                mode.withdrawnReason,
                figures);
    }
}
