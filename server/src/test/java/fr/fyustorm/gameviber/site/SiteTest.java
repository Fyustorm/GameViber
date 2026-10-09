package fr.fyustorm.gameviber.site;

import static fr.fyustorm.gameviber.catalog.CatalogTest.gameviber;
import static fr.fyustorm.gameviber.catalog.CatalogTest.publish;
import static fr.fyustorm.gameviber.catalog.CatalogTest.register;
import static fr.fyustorm.gameviber.catalog.CatalogTest.unique;
import static io.restassured.RestAssured.given;
import static org.hamcrest.Matchers.containsString;
import static org.hamcrest.Matchers.equalTo;
import static org.hamcrest.Matchers.not;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.Map;

import org.junit.jupiter.api.Test;

import fr.fyustorm.gameviber.catalog.CatalogTest;
import io.quarkus.test.junit.QuarkusTest;
import jakarta.ws.rs.core.Response;

@QuarkusTest
public class SiteTest {
    static final String TEMPLATE = "<head>\n<!-- page -->\n<title>Default</title>\n<!-- /page -->\n<script src=\"/site/index.js\"></script>\n</head>";

    @Test
    void theHeadIsThePages() {
        var page = new SitePages.Page("Battles <for> \"Elden\"", "Waves & hits", "/api/modes/abc/image", false, Response.Status.OK);
        String html = SitePages.render(TEMPLATE, page, URI.create("https://gameviber.example/"), "/m/GV-AAAA-BBBB");
        assertFalse(html.contains("Default"));
        assertTrue(html.contains("<title>Battles &lt;for&gt; &quot;Elden&quot; · GameViber</title>"));
        assertTrue(html.contains("<meta name=\"description\" content=\"Waves &amp; hits\" />"));
        assertTrue(html.contains("<meta name=\"robots\" content=\"noindex\" />"));
        assertTrue(html.contains("<meta property=\"og:url\" content=\"https://gameviber.example/m/GV-AAAA-BBBB\" />"));
        assertTrue(html.contains("<meta property=\"og:image\" content=\"https://gameviber.example/api/modes/abc/image\" />"));
        assertTrue(html.endsWith("<!-- /page -->\n<script src=\"/site/index.js\"></script>\n</head>"));

        var steam = SitePages.Page.of("Game modes", "Modes", "https://shared.cloudflare.steamstatic.com/x/header.jpg");
        String withSteam = SitePages.render(TEMPLATE, steam, URI.create("https://gameviber.example/"), "/games/1");
        assertTrue(withSteam.contains("content=\"https://shared.cloudflare.steamstatic.com/x/header.jpg\""));
        assertFalse(withSteam.contains("noindex"));
    }

    @Test
    void longDescriptionsAreCutAtAWord() {
        assertEquals("Short one", SitePages.shorten("  Short\n\none "));
        String cut = SitePages.shorten("word ".repeat(100));
        assertTrue(cut.length() <= SitePages.MAX_DESCRIPTION_CHARS);
        assertTrue(cut.endsWith("word…"));
    }

    @Test
    void aPublicModeHasItsPreview() {
        String token = register(unique("site"));
        String game = unique("Site Game ");
        var detail = publish(token, gameviber(game, null), "public").statusCode(200).extract();
        String id = detail.path("id");
        int gameId = detail.path("game.id");

        given().get("/modes/" + id).then().statusCode(200)
                .contentType(containsString("text/html"))
                .header("Cache-Control", "no-cache")
                .body(containsString("<title>Battle pulse for " + game + " · GameViber</title>"))
                .body(containsString("content=\"Waves in battles\""))
                .body(containsString("/api/modes/" + id + "/image\""))
                .body(not(containsString("noindex")));

        byte[] image = given().get("/api/modes/" + id + "/image").then().statusCode(200)
                .contentType("image/png").header("Cache-Control", containsString("max-age")).extract().asByteArray();
        assertTrue(Arrays.equals(Arrays.copyOf(image, 4), new byte[] { (byte) 0x89, 'P', 'N', 'G' }));
        // Published without its text chunk.
        assertFalse(new String(image, StandardCharsets.ISO_8859_1).contains("tEXt"));

        given().get("/games/" + gameId).then().statusCode(200).body(containsString("<title>" + game + " modes · GameViber</title>"))
                .body(containsString("1 GameViber mode for " + game));
        given().get("/api/games/" + gameId).then().statusCode(200).body("name", equalTo(game)).body("modes", equalTo(1));
    }

    @Test
    void aModeWithoutCapturesFallsBackToItsGame() {
        String token = register(unique("site"));
        String id = publish(token, gameviber(unique("Steam Game "), 4242000L + (long) (Math.random() * 1000), CatalogTest.SCRIPT, Map.of()), "public")
                .statusCode(200).extract().path("id");
        given().get("/api/modes/" + id + "/image").then().statusCode(404);
        given().get("/modes/" + id).then().statusCode(200).body(containsString("steamstatic.com/store_item_assets/steam/apps/"));
    }

    @Test
    void privateModesOnlyByTheirCode() {
        String token = register(unique("site"));
        var detail = publish(token, gameviber(unique("Private Game "), null), "private").statusCode(200).extract();
        String id = detail.path("id");
        String code = detail.path("shareCode");

        given().get("/modes/" + id).then().statusCode(404).body(containsString("noindex"));
        given().get("/api/modes/" + id + "/image").then().statusCode(404);
        given().get("/m/" + code).then().statusCode(200)
                .body(containsString("<title>Battle pulse for "))
                .body(containsString("noindex"))
                .body(containsString("/api/shared/" + code + "/image\""));
        given().get("/api/shared/" + code + "/image").then().statusCode(200).contentType("image/png");
        given().get("/m/GV-NONE-NONE").then().statusCode(404);
    }

    @Test
    void unknownPagesAndTheOthers() {
        given().get("/games/not-a-number").then().statusCode(404).contentType(containsString("text/html"));
        given().get("/games/999999999").then().statusCode(404);
        given().get("/api/games/999999999").then().statusCode(404).body("error", equalTo("no such game"));
        given().get("/games").then().statusCode(200).body(containsString("<title>Community modes · GameViber</title>"));
        given().get("/download").then().statusCode(200).body(containsString("<title>Download · GameViber</title>"));
        // The back-office is still served beside the site.
        given().get("/admin/").then().statusCode(200).body(containsString("GameViber back-office"));
    }
}
