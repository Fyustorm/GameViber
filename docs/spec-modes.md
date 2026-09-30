# GameViber — Spécification des modes (API v1)

Statut : brouillon · Version de l'API : `1`

## 1. Objectif et périmètre

Un **mode** est un script Lua qui transforme ce qui se passe dans le jeu (rumble envoyé
à la manette, inputs du joueur, temps) en consignes pour les jouets connectés à Intiface.
Les modes se créent et se modifient à la volée depuis l'éditeur intégré, sans recompilation.

**Inclus en v1**

- Un seul mode actif à la fois.
- Une seule manette interceptée.
- Sorties Buttplug de type scalaire : `Vibrate`, `Rotate`, `Oscillate`.
- Rechargement à chaud, paramètres réglables depuis la GUI, graphes de debug, simulateur.

**Hors v1** (voir §13) : chaînage de modes, mode par jeu automatique, sorties linéaires
(strokers), multi-manettes, éditeur par blocs.

## 2. Architecture

```
Proxy manette ──► file d'événements ──► Runtime Lua (mode actif) ──► Couche sécurité ──► Sortie Buttplug
 (uinput, FF,        (horodatés)          tick fixe 50 Hz               (non scriptable)     (débit limité)
  boutons, axes)
```

- Le **proxy** produit des événements bruts : effets FF (upload, lecture, arrêt), boutons, axes.
- Le **runtime** normalise ces événements, les distribue aux callbacks du mode, puis appelle
  `tick`. Il tourne dans son propre thread.
- La **couche sécurité** applique le plafond global et le bouton panique, et gère les pertes
  de connexion. Un script ne peut pas la contourner.
- La **sortie** relaie les canaux logiques vers les actionneurs réels, supprime les doublons
  et limite le débit envoyé à Intiface.

## 3. Fichier de mode

- Un fichier = un mode, extension `.luau`, encodage UTF-8.
- Emplacements :
  - modes utilisateur : `~/.config/gameviber/modes/`
  - modes fournis : embarqués dans le binaire, en lecture seule et duplicables depuis l'éditeur.
- Le fichier doit appeler **une seule fois**, au niveau racine, la fonction `mode { ... }` :

```lua
mode {
  api         = 1,                          -- obligatoire
  name        = "Accumulation",             -- obligatoire, affiché dans la GUI
  description = "Chaque vibration ajoute des points...",
  author      = "moi",
  version     = "1.0",
  channels    = { "main" },                 -- canaux de sortie, défaut { "main" }
  params      = { ... },                    -- voir §4
}
```

## 4. Paramètres

Chaque paramètre déclaré dans `params` est affiché automatiquement dans la GUI et lu dans
le script via la table en lecture seule `P`.

| Constructeur | Contrôle GUI | Valeur dans `P` |
|---|---|---|
| `number(default, min, max, label [, step])` | curseur | nombre |
| `bool(default, label)` | case à cocher | booléen |
| `choice(default, { "a", "b", ... }, label)` | liste déroulante | chaîne |
| `button_param(default, label)` | sélecteur de bouton manette | nom de bouton (§6.2) |

```lua
params = {
  per_hit = number(5, 0, 50, "Points par vibration"),
  source  = choice("any", { "any", "rumble", "input" }, "Inactivité ="),
  bonus   = button_param("A", "Bouton bonus"),
}
```

- Les valeurs sont sauvegardées par mode dans `~/.config/gameviber/params/<nom>.toml`.
- Au rechargement, une valeur est conservée si le paramètre garde le même nom et le même
  type. Sinon, elle est remise à son défaut.
- Toute modification depuis la GUI appelle `on_param_changed(name, value)`, si ce callback
  est défini.

## 5. Callbacks

Tous les callbacks sont optionnels, sauf `tick`.

| Callback | Appelé quand |
|---|---|
| `on_start()` | le mode est activé, ou rechargé |
| `on_stop()` | le mode est désactivé, rechargé, ou suspendu après une erreur |
| `tick(dt, input)` | à chaque tick (50 Hz), après la distribution des événements |
| `on_rumble(ev)` | à chaque changement de niveau de rumble |
| `on_rumble_start(ev)` | début d'une vibration (§6.1) |
| `on_rumble_end(ev)` | fin d'une vibration (§6.1) |
| `on_button(ev)` | un bouton est pressé ou relâché |
| `on_param_changed(name, value)` | un paramètre a été modifié dans la GUI |
| `on_device(ev)` | un jouet est connecté ou déconnecté (`ev.connected`, `ev.name`) |

**Ordre d'exécution à chaque tick :**

1. Les événements accumulés depuis le tick précédent sont distribués aux callbacks, dans
   l'ordre chronologique.
2. `tick(dt, input)` est appelé.
3. Les sorties sont figées, puis transmises à la couche sécurité.

## 6. Événements

Tous les événements ont un champ `ev.t`, le temps du moteur en secondes (le même que
`input.time`).

### 6.1 Rumble

Le rumble du jeu est normalisé en 0..1 :

- `strong` : moteur lourd, gauche ;
- `weak` : moteur léger, droit ;
- `level` = `max(strong, weak)`.

Il est calculé à partir de la sémantique force-feedback evdev (effets, durées, gain), comme
dans le prototype.

| Champ | `on_rumble` | `on_rumble_start` | `on_rumble_end` |
|---|---|---|---|
| `strong`, `weak`, `level` | ✓ | ✓ | valeurs à 0 |
| `peak` : level max de la vibration | | | ✓ |
| `duration` : durée en s | | | ✓ |

**Découpage en vibrations**

- Une vibration commence quand `level` passe au-dessus de `rumble_threshold` (défaut 0.05).
- Elle se termine quand `level` reste sous ce seuil pendant `rumble_release` (défaut 80 ms).
  Ce délai évite de découper en plusieurs vibrations une rafale d'effets très courts.
- Ces deux réglages sont globaux dans la GUI. Un mode peut les surcharger dans `mode { }` :
  `rumble_threshold = 0.1`, `rumble_release = 0.15`.

### 6.2 Boutons

`on_button(ev)` reçoit :

- `ev.button` : nom normalisé, disposition Xbox ;
- `ev.pressed` : `true` à l'appui, `false` au relâchement.

Boutons disponibles :

```
A B X Y  LB RB  BACK START GUIDE  LS RS  DPAD_UP DPAD_DOWN DPAD_LEFT DPAD_RIGHT
```

Le d-pad est converti en boutons même quand le driver l'expose en axes (hat). Les
gâchettes restent des axes (§7), mais `LT` et `RT` génèrent aussi un événement bouton
quand elles passent au-dessus ou en dessous de 0.5.

Les mouvements d'axes ne génèrent pas de callback, pour éviter un flot d'événements. Leur
état courant se lit dans `input.axes` à chaque tick.

## 7. La table `input` (état courant, lecture seule)

```lua
input.time               -- s depuis l'activation du mode
input.rumble.strong      -- 0..1
input.rumble.weak        -- 0..1
input.rumble.level       -- max(strong, weak)
input.rumble.avg         -- (strong + weak) / 2
input.rumble.active      -- true pendant une vibration (§6.1)
input.buttons.A          -- true si maintenu (idem pour chaque bouton §6.2)
input.axes.LX, LY, RX, RY  -- -1..1 (zone morte 0.1 appliquée)
input.axes.LT, RT        -- 0..1
input.rumble_idle        -- s depuis la fin de la dernière vibration (0 si active)
input.input_idle         -- s depuis le dernier input joueur (bouton, ou axe hors zone morte)
input.idle               -- min(rumble_idle, input_idle)
```

## 8. Sorties

### 8.1 Canaux

Le script ne connaît pas les jouets. Il écrit sur des **canaux logiques**, déclarés dans
`channels`. La GUI associe à chaque canal un ou plusieurs actionneurs réels, par exemple
`main` vers le vibreur 1 du Lush et `aux` vers la rotation du Nora. Un actionneur qui
n'est associé à aucun canal reste à 0.

### 8.2 Fonctions

```lua
set(x [, channel])            -- niveau de base du canal, 0..1, maintenu jusqu'au prochain set
pulse(x, seconds [, channel]) -- surimpression temporaire d'intensité x pendant `seconds`
play(pattern [, opts])        -- joue un motif (§8.3), renvoie un handle avec :stop()
stop_all()                    -- remet le niveau de base à 0 et annule pulses et motifs
```

- `channel` vaut `"main"` par défaut, ou `"*"` pour tous les canaux.
- Les valeurs hors de 0..1 sont bornées silencieusement.
- **Valeur finale d'un canal** = `max(niveau de base, pulses actifs, motifs actifs)`.
- Le type d'actionneur (vibration, rotation, oscillation) est choisi dans la GUI au moment
  du routage. Pour le script, c'est toujours une intensité entre 0 et 1.

### 8.3 Motifs

```lua
local heartbeat = pattern {
  { 0.00, 0.8 }, { 0.10, 0.0 }, { 0.20, 0.6 }, { 0.30, 0.0 }, { 0.80, 0.0 },
}  -- liste de { temps en s, intensité }, interpolée linéairement

play(heartbeat, { channel = "main", loops = 3, scale = 0.5 })  -- loops = 0 : boucle infinie
```

## 9. Utilitaires

| Fonction | Rôle |
|---|---|
| `plot(name, value)` | trace une courbe dans le panneau debug de l'éditeur |
| `log(...)` | écrit dans la console de l'éditeur (`print` est redirigé ici) |
| `after(seconds, fn)` | appelle `fn` une fois après le délai, renvoie un handle `:cancel()` |
| `every(seconds, fn)` | appelle `fn` périodiquement, renvoie un handle `:cancel()` |
| `clamp(x, a, b)`, `lerp(a, b, t)`, `map(x, a1, b1, a2, b2)` | maths courantes |
| `random([a, b])` | aléatoire, graine réinitialisée à chaque `on_start` |
| `persist` | table conservée entre deux rechargements à chaud (pas entre deux lancements) |

Les minuteurs (`after`, `every`) sont évalués au début de chaque tick, avant les événements.
Leur résolution est donc de 20 ms.

## 10. Exécution et sandbox

- Moteur : **Luau** via `mlua`, en mode sandbox.
- Bibliothèques disponibles : `math`, `string`, `table`, `bit32`, `utf8`.
- Bibliothèques absentes : `io`, `os`, `require`, `load`, `debug`, et tout accès fichier ou
  réseau.
- Budget par appel de callback : 100 000 instructions, contrôlées par l'interruption Luau.
  Un dépassement compte comme une erreur d'exécution.
- Mémoire du mode : 16 Mo maximum.
- Les variables globales du script sont réinitialisées à chaque (re)chargement, sauf
  `persist`.

## 11. Erreurs et rechargement à chaud

- L'éditeur sauvegarde, et un watcher détecte aussi les modifications faites dans un
  éditeur externe. Chaque sauvegarde déclenche un rechargement.
- **Erreur de chargement** (syntaxe, `mode {}` absent ou invalide) : l'ancienne version du
  mode continue de tourner, et l'erreur s'affiche avec son numéro de ligne.
- **Rechargement réussi** : `on_stop` est appelé sur l'ancienne version, puis `on_start`
  sur la nouvelle. Les valeurs de paramètres compatibles et `persist` sont conservées.
- **Erreur d'exécution** dans un callback : toutes les sorties passent immédiatement à 0,
  le mode est suspendu, et l'erreur s'affiche avec la trace. Le bouton « Reprendre »
  relance `on_start`.

## 12. Sécurité (hors script)

- **Plafond global** d'intensité, réglable dans la GUI : défaut 1.0, appliqué après le mode.
- **Bouton panique** : BACK + START maintenus 0.5 s (combo configurable).
  - Il coupe tous les jouets et suspend le mode jusqu'à la réactivation depuis la GUI.
  - Le combo n'est pas transmis aux callbacks tant qu'il est maintenu.
- **Perte de source** : si la manette est déconnectée ou le proxy arrêté, toutes les
  sorties passent à 0.
- **Débit de sortie** : 20 envois/s maximum par jouet. Un changement de moins de 0.01
  n'est pas envoyé, sauf le passage à 0, toujours envoyé.

## 13. Évolutions prévues (hors v1)

- **Chaînage** : `input.upstream` exposerait la sortie du mode précédent.
- **Mode par jeu** : détection du process du jeu et association à un mode.
- **Sorties linéaires** (`LinearCmd`, pour les strokers) : `stroke(speed, range)`.
- **Multi-manettes** : `ev.pad` et `input.pads[i]`.
- **Éditeur par blocs** qui générerait du Luau.

## 14. Exemples

### 14.1 Simple (parité GHR, mode par défaut)

```lua
mode {
  api = 1,
  name = "Simple",
  description = "Relaie le rumble du jeu, comme le Game Haptics Router.",
  params = {
    combine    = choice("avg", { "avg", "max" }, "Combinaison des moteurs"),
    multiplier = number(1, 0, 5, "Multiplicateur", 0.1),
    baseline   = number(0, 0, 1, "Vibration minimale", 0.01),
  },
}

function tick(dt, input)
  local r = input.rumble
  local level = (P.combine == "max") and r.level or r.avg
  if level == 0 and P.baseline == 0 then
    set(0)
  else
    set(math.max(level * P.multiplier, P.baseline))
  end
end
```

### 14.2 Accumulation

```lua
mode {
  api = 1,
  name = "Accumulation",
  description = "Chaque vibration et chaque appui sur le bouton bonus ajoutent des points. "
             .. "Le rumble du jeu est proportionnel aux points, qui fondent sans action.",
  params = {
    per_hit   = number(5, 0, 50, "Points par vibration"),
    bonus     = button_param("A", "Bouton bonus"),
    per_press = number(1, 0, 10, "Points par appui bonus"),
    decay     = number(2, 0, 20, "Perte par seconde"),
    idle      = number(1.5, 0, 10, "Délai avant perte (s)"),
    idle_src  = choice("any", { "any", "rumble", "input" }, "Inactivité ="),
    max       = number(100, 10, 500, "Points max", 5),
    floor     = number(0.2, 0, 1, "Fond continu à points max", 0.05),
  },
}

persist.points = persist.points or 0

local function add(n)
  persist.points = clamp(persist.points + n, 0, P.max)
end

function on_rumble_start(ev)
  add(P.per_hit)
end

function on_button(ev)
  if ev.pressed and ev.button == P.bonus then
    add(P.per_press)
  end
end

function tick(dt, input)
  local idle = ({ any = input.idle, rumble = input.rumble_idle, input = input.input_idle })[P.idle_src]
  if idle > P.idle then
    add(-P.decay * dt)
  end

  local ratio = persist.points / P.max
  set(math.max(input.rumble.level * ratio, P.floor * ratio))

  plot("points", persist.points)
  plot("sortie", ratio)
end
```
