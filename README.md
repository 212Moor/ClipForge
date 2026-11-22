# ClipForge

Application de bureau Windows 10/11 (Tauri 2 + Rust + React/TypeScript) qui transforme une rediffusion Twitch (VOD) en clips verticaux 9:16 prêts pour TikTok, en privilégiant les signaux locaux avant d'utiliser l'API Gemini de façon très économe en tokens.

## Fonctionnalités complètes (Parties 1, 2 & 3)

1. **Fondations & Sidecars vérifiés** : Tauri 2 + effet Mica/Acrylic Windows 11/10, SQLite WAL (9 tables migrées), logs JSON `tracing`, téléchargement reprenable + vérification SHA-256 avant toute exécution (`FFmpeg`, `yt-dlp`, `TwitchDownloaderCLI`, `whisper.cpp` CUDA/Vulkan/CPU), auto-test avec rollback, import hors ligne, clé API Gemini dans le Gestionnaire d'identifiants Windows (`keyring`).
2. **Analyse locale (0 token) & Gemini économe** :
   - Ingestion audio seul + chat JSON + détection des zones mutées (jamais la VOD vidéo complète).
   - Signaux locaux par pas de 5 s (chat, emotes, événements, énergie audio ~EBU R128, VAD vocal), scoring robuste médiane/MAD, dédoublonnage (distance min 90 s).
   - Transcription locale `whisper.cpp` uniquement sur les fenêtres candidates (+15 s).
   - Cascade Gemini Étage A (lots de 12, thinking minimal) + Étage B (top K) + mode Éco Batch API (-50 %), cache disque SQLite SHA-256 (0 token si relancé), plafond dur configurable, `pricing.json` éditable, limiteur RPM/TPM, Timeline Canvas HiDPI et tableau de bord Coûts.
3. **Revue des clips & Rendu 9:16** :
   - Grille 9:16 avec raccourcis clavier (`←` rejeter, `→` valider, `E` éditer).
   - Tiroir d'édition : découpe avec lecture en boucle, mises en page **Split / Fit / Fill**, zone facecam mémorisée par streamer, **5 styles de sous-titres ASS mot à mot** (police et couleurs éditables, zones de sécurité TikTok), accroche 2–3 s, régénération ciblée Gemini sur un seul clip.
   - Moteur FFmpeg matériel (`NVENC` → `AMF` → `QSV` → `libx264` auto), téléchargement vidéo des **seuls segments retenus** (±5 s), export **1080×1920**, H.264 `yuv420p` `+faststart`, AAC 192 kb/s, **loudnorm −14 LUFS**, fichiers `MP4` + `.txt` + `clips_export.csv`.

## Prérequis Windows 10/11

1. **Visual Studio Build Tools 2022** — charge *Développement Desktop en C++* (`MSVC v143` + `Windows 10/11 SDK`).
2. **Rust (`rustup`)** `1.85+` — cible `x86_64-pc-windows-msvc`.
3. **Node.js** `20.x` ou `22.x` LTS.
4. **Microsoft Edge WebView2 Runtime** (préinstallé sur Windows 11).

## Lancer en développement et tester en 60 secondes

```powershell
# 1. Tests unitaires Rust (migrations, scoring, budget, cache, schémas Gemini, ASS, FFmpeg)
cargo test --manifest-path src-tauri/Cargo.toml

# 2. Dépendances front et lancement
npm install
npm run tauri dev
```

Pour tester tout le pipeline sans clé API ni VOD Twitch :
1. Vérifiez que **FFmpeg** est installé depuis l'écran **Premier lancement**.
2. Sur **Accueil**, cliquez sur **« Tester avec la démo 10 min (0 token) »**.
3. Dans **Analyse**, cliquez sur **« Local seul (0 tok) »**.
4. Consultez la **Timeline**, puis ouvrez **Revue des clips** → **« Créer des clips depuis les candidats locaux »** → validez un clip (`→`) ou éditez-le (`E`).
5. Ouvrez **File d'export** → **« Exporter MP4 »** pour générer un vrai MP4 1080×1920 avec sous-titres ASS mot à mot, loudnorm −14 LUFS, `.txt` et `clips_export.csv`.

## Fichiers de configuration (dans `%LOCALAPPDATA%\ClipForge\`)

- `pricing.json` : tarifs USD par million de tokens et taux EUR/USD (jamais codés en dur).
- `bin/<outil>/<version>/` : sidecars vérifiés par SHA-256.
- `models/` : modèles `whisper.cpp` téléchargés à la demande.
- `projects/<project_id>/exports/` : MP4 exportés, légendes `.txt` et `clips_export.csv`.
- `logs/` : journaux structurés JSON quotidiens (aucune clé API journalisée).

## Build installeur NSIS & mises à jour signées

1. Remplacez `VOTRE_COMPTE/clipforge` dans `src-tauri/tauri.conf.json` (`plugins.updater.endpoints`).
2. Synchronisez les empreintes SHA-256 des sidecars :
   ```powershell
   powershell -ExecutionPolicy Bypass -File .\scripts\sync-sidecar-manifest.ps1
   ```
3. Générez la clé de l'updater Tauri :
   ```powershell
   npx tauri signer generate -w "$env:USERPROFILE\.tauri\clipforge.key"
   ```
   Placez la clé publique dans `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`) et la clé privée dans les secrets GitHub Actions (`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`).
4. Build local de l'installeur NSIS :
   ```powershell
   npm run tauri build -- --bundles nsis
   ```
