package fr.fyustorm.gameviber.catalog;

import static io.restassured.RestAssured.given;
import static org.hamcrest.Matchers.containsString;
import static org.hamcrest.Matchers.equalTo;
import static org.hamcrest.Matchers.hasItem;
import static org.hamcrest.Matchers.not;
import static org.hamcrest.Matchers.nullValue;
import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.UUID;
import java.util.zip.CRC32;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

import org.junit.jupiter.api.Test;

import io.quarkus.test.junit.QuarkusTest;
import io.restassured.http.ContentType;
import io.restassured.specification.RequestSpecification;

@QuarkusTest
public class CatalogTest {
    public static final String SCRIPT = "mode { api = 1, name = \"Battles\" }\nfunction tick(dt, input) set(input.rumble.level) end\n";

    // --- helpers

    public static String unique(String prefix) {
        return prefix + UUID.randomUUID().toString().substring(0, 8);
    }

    public static String register(String pseudo) {
        return given().contentType(ContentType.JSON).body(Map.of("pseudo", pseudo, "password", "secret-password"))
                .post("/api/authors").then().statusCode(200).extract().path("token");
    }

    static RequestSpecification as(String token) {
        return given().header("Authorization", "Bearer " + token);
    }

    static String adminToken() {
        return given().formParam("token", "test-admin").post("/api/admin/session").then().statusCode(200).extract().path("token");
    }

    static byte[] chunk(String type, byte[] data) {
        var crc = new CRC32();
        crc.update(type.getBytes(StandardCharsets.US_ASCII));
        crc.update(data);
        return ByteBuffer.allocate(12 + data.length).putInt(data.length).put(type.getBytes(StandardCharsets.US_ASCII)).put(data).putInt((int) crc.getValue()).array();
    }

    /** A 1x1 PNG with a text chunk that must not survive. */
    static byte[] png() {
        var out = new ByteArrayOutputStream();
        out.writeBytes(new byte[] { (byte) 0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n' });
        out.writeBytes(chunk("IHDR", new byte[] { 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0 }));
        out.writeBytes(chunk("tEXt", "Author\0Someone".getBytes(StandardCharsets.ISO_8859_1)));
        out.writeBytes(chunk("IDAT", new byte[] { 0x78, (byte) 0x9c, 0x63, 0, 1, 0, 0, 5, 0, 1 }));
        out.writeBytes(chunk("IEND", new byte[0]));
        return out.toByteArray();
    }

    static byte[] zip(Map<String, byte[]> entries) {
        var out = new ByteArrayOutputStream();
        try (var zip = new ZipOutputStream(out)) {
            for (var e : entries.entrySet()) {
                zip.putNextEntry(new ZipEntry(e.getKey()));
                zip.write(e.getValue());
                zip.closeEntry();
            }
        } catch (IOException e) {
            throw new IllegalStateException(e);
        }
        return out.toByteArray();
    }

    public static byte[] gameviber(String game, Long app, String script, Map<String, byte[]> more) {
        String appJson = app == null ? "null" : app.toString();
        String manifest = "{\"format\":2,\"app_version\":\"0.1.0\",\"mode\":\"battles\",\"game\":{\"name\":\"" + game + "\",\"steam_app_id\":" + appJson
                + "},\"inputs\":{\"captures\":[{\"file\":\"battle-1.png\",\"phase\":\"battle\"}]}}";
        Map<String, byte[]> entries = new LinkedHashMap<>();
        entries.put("gameviber.json", manifest.getBytes(StandardCharsets.UTF_8));
        entries.put("mode.luau", script.getBytes(StandardCharsets.UTF_8));
        entries.putAll(more);
        return zip(entries);
    }

    public static byte[] gameviber(String game, Long app) {
        byte[] funscript = "{\"metadata\": {\"creator\": \"x\"}, \"actions\": [{\"at\": 0, \"pos\": 10}, {\"at\": 400, \"pos\": 90}]}"
                .getBytes(StandardCharsets.UTF_8);
        return gameviber(game, app, SCRIPT, Map.of("variants/boss.luau", SCRIPT.getBytes(StandardCharsets.UTF_8), "captures/battle-1.png", png(),
                "funscripts/hit.funscript", funscript));
    }

    public static io.restassured.response.ValidatableResponse publish(String token, byte[] file, String visibility) {
        return as(token).multiPart("package", "mode.gameviber", file, "application/zip")
                .multiPart("name", "Battle pulse").multiPart("description", "Waves in battles")
                .multiPart("visibility", visibility).multiPart("changelog", "First version")
                .post("/api/modes").then();
    }

    // --- tests

    @Test
    void authorsRegisterAndSignIn() {
        String pseudo = unique("fyu");
        register(pseudo);
        given().contentType(ContentType.JSON).body(Map.of("pseudo", pseudo.toUpperCase(), "password", "another-password"))
                .post("/api/authors").then().statusCode(409).body("error", equalTo("this pseudo is taken"));
        given().contentType(ContentType.JSON).body(Map.of("pseudo", pseudo, "password", "wrong-password"))
                .post("/api/authors/session").then().statusCode(401);
        given().contentType(ContentType.JSON).body(Map.of("pseudo", pseudo.toUpperCase(), "password", "secret-password"))
                .post("/api/authors/session").then().statusCode(200).body("pseudo", equalTo(pseudo));
        given().contentType(ContentType.JSON).body(Map.of("pseudo", "x", "password", "secret-password"))
                .post("/api/authors").then().statusCode(400);
        given().contentType(ContentType.JSON).body(Map.of("pseudo", unique("ok"), "password", "short"))
                .post("/api/authors").then().statusCode(400);
    }

    @Test
    void aPrivateModeIsSharedByItsCodeThenListedOncePublic() throws Exception {
        String game = unique("Hades ");
        String token = register(unique("a"));
        var detail = publish(token, gameviber(game, null), "private").statusCode(200)
                .body("visibility", equalTo("private")).body("versions[0].number", equalTo(1)).body("game.name", equalTo(game))
                .extract();
        String id = detail.path("id");
        String code = detail.path("shareCode");
        assertTrue(code.matches("GV-[A-Z2-9]{4}-[A-Z2-9]{4}"), code);

        // Private: not listed, not found by its id, found by its code (whatever its case).
        given().get("/api/games").then().body("name", not(hasItem(game)));
        given().get("/api/modes/" + id).then().statusCode(404);
        given().get("/api/shared/" + code.toLowerCase()).then().statusCode(200).body("name", equalTo("Battle pulse")).body("shareCode", nullValue());
        byte[] file = given().get("/api/shared/" + code + "/package").then().statusCode(200).extract().asByteArray();
        Map<String, byte[]> entries = SharedPackage.entries(file);
        assertTrue(entries.containsKey("variants/boss.luau"), "variants travel with the mode");
        byte[] png = entries.get("captures/battle-1.png");
        assertFalse(new String(png, StandardCharsets.ISO_8859_1).contains("tEXt"), "metadata stripped");
        assertArrayEquals(SharedPackage.stripPng("x", png), png, "still a valid PNG");
        String funscript = new String(entries.get("funscripts/hit.funscript"), StandardCharsets.UTF_8);
        assertFalse(funscript.contains("creator"), "only its actions");
        assertTrue(funscript.contains("\"pos\":90"), funscript);

        // Public: listed, its downloads counted.
        as(token).contentType(ContentType.JSON).body(Map.of("visibility", "public")).patch("/api/modes/" + id).then().statusCode(200);
        int gameId = given().queryParam("search", game.toUpperCase()).get("/api/games").then().statusCode(200)
                .body("[0].name", equalTo(game)).body("[0].modes", equalTo(1)).extract().path("[0].id");
        given().get("/api/games/" + gameId + "/modes?sort=downloads").then().body("[0].id", equalTo(id)).body("[0].downloads", equalTo(1));
        given().queryParam("name", game.toLowerCase()).get("/api/games/match").then().statusCode(200).body("id", equalTo(gameId)).body("modes", equalTo(1));
        given().queryParam("name", "No such game").get("/api/games/match").then().statusCode(404);
        given().get("/api/modes/" + id + "/package").then().statusCode(200).header("Content-Disposition", containsString(id + "-1.gameviber"));
        given().get("/api/modes/" + id).then().body("downloads", equalTo(2)).body("visibility", nullValue());

        // A new share code: the old one no longer works.
        String newCode = as(token).post("/api/modes/" + id + "/share-code").then().statusCode(200).extract().path("shareCode");
        given().get("/api/shared/" + code).then().statusCode(404);
        given().get("/api/shared/" + newCode).then().statusCode(200);
    }

    @Test
    void versionsAreForTheSameGame() {
        String token = register(unique("a"));
        String id = publish(token, gameviber("Elden Ring", 1245620L), "public").statusCode(200).extract().path("id");
        as(token).multiPart("package", "m.gameviber", gameviber("Hades II", null), "application/zip").post("/api/modes/" + id + "/versions")
                .then().statusCode(400).body("error", containsString("another game"));
        // The same game by its app id, whatever its name.
        as(token).multiPart("package", "m.gameviber", gameviber("ELDEN RING™", 1245620L), "application/zip").multiPart("changelog", "Faster")
                .post("/api/modes/" + id + "/versions").then().statusCode(200)
                .body("versions[0].number", equalTo(2)).body("versions[0].changelog", equalTo("Faster"));
        given().get("/api/modes/" + id + "/package?version=1").then().statusCode(200).header("Content-Disposition", containsString("-1.gameviber"));
        given().get("/api/modes/" + id + "/package?version=9").then().statusCode(404);
        // Someone else's mode.
        as(register(unique("b"))).contentType(ContentType.JSON).body(Map.of("name", "Mine")).patch("/api/modes/" + id).then().statusCode(403);
        given().get("/api/modes/" + id).then().body("name", equalTo("Battle pulse"));
    }

    @Test
    void packagesAreChecked() {
        String token = register(unique("a"));
        publish(token, "not a zip".getBytes(StandardCharsets.UTF_8), "public").statusCode(400).body("error", containsString("not a file of a shared mode"));
        publish(token, zip(Map.of("mode.luau", SCRIPT.getBytes(StandardCharsets.UTF_8))), "public").statusCode(400).body("error", containsString("missing"));
        publish(token, gameviber("Game", null, SCRIPT.replace("api = 1", "api = 9"), Map.of()), "public").statusCode(400).body("error", containsString("mode API 9"));
        publish(token, gameviber("Game", null, SCRIPT, Map.of("captures/../evil.png", png())), "public").statusCode(400).body("error", containsString("unexpected file"));
        publish(token, gameviber("Game", null, SCRIPT, Map.of("captures/x.png", "nope".getBytes(StandardCharsets.UTF_8))), "public").statusCode(400)
                .body("error", containsString("not a PNG"));
        publish(token, gameviber("Game", null, SCRIPT, Map.of("funscripts/hit.funscript", "{\"actions\": [{\"at\": 0}]}".getBytes(StandardCharsets.UTF_8))), "public")
                .statusCode(400).body("error", containsString("not a funscript"));
        publish(token, gameviber("", null), "public").statusCode(400).body("error", containsString("names no game"));
        publish(token, gameviber("Game", null), "everyone").statusCode(400);
        given().multiPart("package", "m.gameviber", gameviber("Game", null), "application/zip").post("/api/modes").then().statusCode(401);
    }

    @Test
    void authorsWithdrawTheirModes() {
        String token = register(unique("a"));
        String id = publish(token, gameviber(unique("G"), null), "public").statusCode(200).extract().path("id");
        as(token).delete("/api/modes/" + id).then().statusCode(200).body("withdrawnReason", equalTo("withdrawn by its author"));
        given().get("/api/modes/" + id).then().statusCode(404);
        as(token).get("/api/authors/me/modes").then().statusCode(200).body("id", hasItem(id));
    }

    @Test
    void administratorsWithdrawBlockMergeAndHandleReports() {
        given().formParam("token", "wrong").post("/api/admin/session").then().statusCode(401);
        given().get("/api/admin/modes").then().statusCode(401);
        String admin = adminToken();
        String pseudo = unique("a");
        String token = register(pseudo);
        String game = unique("Game ");
        String id = publish(token, gameviber(game, null), "public").statusCode(200).extract().path("id");

        // A report, handled.
        given().contentType(ContentType.JSON).body(Map.of("reason", "broken", "details", "since patch 1.04")).post("/api/modes/" + id + "/reports").then().statusCode(204);
        given().contentType(ContentType.JSON).body(Map.of("reason", "spam")).post("/api/modes/" + id + "/reports").then().statusCode(400);
        int report = as(admin).get("/api/admin/reports").then().statusCode(200).body("mode", hasItem(id)).extract()
                .path("find { it.mode == '" + id + "' }.id");
        as(admin).post("/api/admin/reports/" + report + "/resolve").then().statusCode(204);
        as(admin).get("/api/admin/reports").then().body("id", not(hasItem(report)));

        // Withdrawn, restored.
        as(admin).formParam("reason", "not a mode").post("/api/admin/modes/" + id + "/withdraw").then().statusCode(200);
        given().get("/api/modes/" + id).then().statusCode(404);
        as(admin).post("/api/admin/modes/" + id + "/restore").then().statusCode(200);
        given().get("/api/modes/" + id).then().statusCode(200);

        // Two names of one game; a Steam app id of this run's own, the test database outlives runs.
        int steamAppId = 1_000_000 + Math.floorMod(UUID.randomUUID().hashCode(), 1_000_000_000);
        String other = publish(token, gameviber(game + " Remastered", (long) steamAppId), "public").statusCode(200).extract().path("id");
        int from = given().get("/api/modes/" + other).then().extract().path("game.id");
        int into = given().get("/api/modes/" + id).then().extract().path("game.id");
        as(admin).post("/api/admin/games/" + from + "/merge?into=" + into).then().statusCode(200).body("modes", equalTo(2)).body("steamAppId", equalTo(steamAppId));
        given().get("/api/modes/" + other).then().body("game.id", equalTo(into));

        // Blocked: no more signing in nor publishing.
        int author = as(admin).get("/api/admin/authors").then().extract().path("find { it.pseudo == '" + pseudo + "' }.id");
        as(admin).post("/api/admin/authors/" + author + "/block").then().statusCode(200).body("blocked", equalTo(true));
        given().contentType(ContentType.JSON).body(Map.of("pseudo", pseudo, "password", "secret-password")).post("/api/authors/session").then().statusCode(403);
        publish(token, gameviber(game, null), "public").statusCode(403);
        assertEquals(2, (int) as(admin).get("/api/admin/authors").then().extract().path("find { it.pseudo == '" + pseudo + "' }.modes"));
    }
}
