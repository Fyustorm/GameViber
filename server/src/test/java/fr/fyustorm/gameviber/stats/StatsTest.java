package fr.fyustorm.gameviber.stats;

import static fr.fyustorm.gameviber.catalog.CatalogTest.gameviber;
import static fr.fyustorm.gameviber.catalog.CatalogTest.publish;
import static fr.fyustorm.gameviber.catalog.CatalogTest.register;
import static fr.fyustorm.gameviber.catalog.CatalogTest.unique;
import static io.restassured.RestAssured.given;
import static org.hamcrest.Matchers.contains;
import static org.hamcrest.Matchers.containsString;
import static org.hamcrest.Matchers.equalTo;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;
import java.util.Map;

import org.junit.jupiter.api.Test;

import io.quarkus.test.junit.QuarkusTest;
import io.restassured.http.ContentType;

@QuarkusTest
class StatsTest {
    static void play(String installation, String mode, long seconds, int sessions) {
        given().contentType(ContentType.JSON)
                .body(Map.of("installation", installation, "plays", List.of(Map.of("mode", mode, "seconds", seconds, "sessions", sessions))))
                .post("/api/stats/plays").then().statusCode(204);
    }

    static io.restassured.response.ValidatableResponse vote(String installation, String mode, int value) {
        return given().contentType(ContentType.JSON).body(Map.of("installation", installation, "mode", mode, "value", value)).post("/api/stats/votes").then();
    }

    static String installation() {
        return unique("installation-") + "-0123456789";
    }

    @Test
    void modesAreRankedByWhoKeepsPlayingAndLikingThem() {
        String token = register(unique("a"));
        String game = unique("Ranked ");
        String loved = publish(token, gameviber(game, null), "public").statusCode(200).extract().path("id");
        String tried = publish(token, gameviber(game, null), "public").statusCode(200).extract().path("id");
        String fresh = publish(token, gameviber(game, null), "public").statusCode(200).extract().path("id");

        // Three players keep playing the first; one tries the second once.
        for (int i = 0; i < 3; i++) {
            String player = installation();
            play(player, loved, 1800, 2);
            play(player, loved, 1200, 1);
            vote(player, loved, 1).statusCode(204);
        }
        String once = installation();
        play(once, tried, 300, 1);
        vote(once, tried, -1).statusCode(204);
        vote(once, tried, 1).statusCode(204);
        vote(once, tried, 0).statusCode(204);

        given().get("/api/modes/" + loved).then().statusCode(200)
                .body("figures.players", equalTo(3)).body("figures.medianMinutes", equalTo(50))
                .body("figures.cameBack", equalTo(1.0f)).body("figures.likes", equalTo(3)).body("figures.dislikes", equalTo(0));
        given().get("/api/modes/" + tried).then().body("figures.likes", equalTo(0)).body("figures.dislikes", equalTo(0)).body("figures.cameBack", equalTo(0.0f));

        int gameId = given().get("/api/modes/" + loved).then().extract().path("game.id");
        given().get("/api/games/" + gameId + "/modes").then().body("id", contains(loved, tried, fresh));
        given().get("/api/games/" + gameId + "/modes?sort=rating").then().body("[0].id", equalTo(loved));
        given().get("/api/games/" + gameId + "/modes?sort=played").then().body("[0].id", equalTo(loved));
        given().get("/api/games/" + gameId + "/modes?sort=new").then().body("[0].id", equalTo(fresh));
    }

    @Test
    void onlyPlayersVoteAndReportsAreChecked() {
        String token = register(unique("a"));
        String mode = publish(token, gameviber(unique("G"), null), "public").statusCode(200).extract().path("id");
        vote(installation(), mode, 1).statusCode(403).body("error", containsString("play it first"));
        given().contentType(ContentType.JSON).body(Map.of("installation", "short", "plays", List.of())).post("/api/stats/plays").then().statusCode(400);
        vote(installation(), mode, 5).statusCode(400);
        // Unknown modes are skipped; a day counts 24 hours at most.
        String player = installation();
        given().contentType(ContentType.JSON)
                .body(Map.of("installation", player, "plays", List.of(Map.of("mode", "nope", "seconds", 10, "sessions", 1), Map.of("mode", mode, "seconds", 999999, "sessions", 1))))
                .post("/api/stats/plays").then().statusCode(204);
        given().get("/api/modes/" + mode).then().body("figures.medianMinutes", equalTo(24 * 60));
    }

    @Test
    void fewVotesCountLittle() {
        assertEquals(0, Stats.wilson(0, 0));
        assertTrue(Stats.wilson(1, 1) < Stats.wilson(90, 100), "one like is less sure than 90 of 100");
        assertTrue(Stats.wilson(90, 100) < Stats.wilson(900, 1000));
    }
}
