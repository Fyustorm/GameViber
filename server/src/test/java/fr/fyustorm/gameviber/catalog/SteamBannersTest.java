package fr.fyustorm.gameviber.catalog;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;

import org.junit.jupiter.api.Test;

import com.fasterxml.jackson.databind.ObjectMapper;

class SteamBannersTest {
    private final ObjectMapper json = new ObjectMapper();

    @Test
    void theBannerIsReadFromSteamsAnswer() throws Exception {
        String url = "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/4078430/97215eb6/header.jpg?t=1791388584";
        var answer = json.readTree("{\"4078430\": {\"success\": true, \"data\": {\"header_image\": \"" + url + "\"}}}");
        assertEquals(url, SteamBanners.headerOf(answer, 4078430));
        assertNull(SteamBanners.headerOf(answer, 1));
        assertNull(SteamBanners.headerOf(json.readTree("{\"1\": {\"success\": false}}"), 1));
        // Only Steam's own images.
        assertNull(SteamBanners.headerOf(json.readTree("{\"1\": {\"success\": true, \"data\": {\"header_image\": \"https://evil.example/x.steamstatic.com.jpg\"}}}"), 1));
        assertNull(SteamBanners.headerOf(json.readTree("{\"1\": {\"success\": true, \"data\": {\"header_image\": \"http://cdn.steamstatic.com/x.jpg\"}}}"), 1));
    }

    @Test
    void aGameWithoutABannerAskedShowsThePathWithoutAHash() {
        var game = new Game();
        assertNull(SteamBanners.of(game));
        game.changeSteamApp(1245620L);
        assertEquals("https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/1245620/header.jpg", SteamBanners.of(game));
        game.bannerUrl = "https://shared.akamai.steamstatic.com/a/header.jpg";
        assertEquals(game.bannerUrl, SteamBanners.of(game));
        game.changeSteamApp(1L);
        assertNull(game.bannerUrl);
    }
}
