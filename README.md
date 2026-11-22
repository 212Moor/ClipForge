# ClipForge

Application de bureau Windows 10/11 (Tauri 2, Rust, React et TypeScript) qui transforme une rediffusion Twitch en clips verticaux 9:16 pour TikTok. Le traitement local passe en premier ; l'API Gemini ne sert qu'au jugement éditorial, avec un budget de tokens maîtrisé.

**État du projet : phase 1 sur 3 (fondations).** Sont fonctionnels : fenêtre Windows 11 (Mica ou Acrylic), navigation, thèmes et langues (FR/EN), palette de commandes, gestionnaire d'outils externes (sidecars), clé API Gemini dans le Gestionnaire d'identifiants Windows, base SQLite migrée, journaux structurés. Les écrans Accueil, Analyse, Timeline, Revue, Export et Coûts sont des coquilles : ils seront branchés avec l'ingestion, l'analyse Gemini et le rendu.

## Prérequis (Windows)

1. **Visual Studio Build Tools 2022** avec la charge de travail « Développement Desktop en C++ » (MSVC v143 et SDK Windows).
2. **Rust** via `rustup` (1.85 ou plus récent), cible `x86_64-pc-windows-msvc`. Lancez `rustup update`.
3. **Node.js** 20 ou 22 LTS.
4. **WebView2** (déjà présent sur Windows 11).

## Premier démarrage

```powershell
npm install
powershell -ExecutionPolicy Bypass -File .\scripts\sync-sidecar-manifest.ps1
npm run tauri dev
```

Le script de synchronisation télécharge les versions officielles de FFmpeg, yt-dlp, TwitchDownloaderCLI et whisper.cpp pour en calculer les empreintes SHA-256 (comptez plusieurs centaines de Mo, dont l'archive CUDA de whisper.cpp ; ajoutez `-SkipWhisperCuda` pour l'exclure). Sans lui, l'écran **Premier lancement** refuse d'installer les outils, par sécurité.

Ensuite, dans l'application : **Premier lancement** pour installer les 4 outils, puis **Paramètres** pour enregistrer votre clé API.

### Clé API Gemini : abonnement Pro et API sont deux choses distinctes

L'abonnement Google AI Pro ne donne pas accès à l'API développeur. Créez une clé sur [Google AI Studio](https://aistudio.google.com/apikey) : palier gratuit (quotas limités, données soumises aux conditions de Google) ou palier facturé à l'usage. La clé est stockée dans le Gestionnaire d'identifiants Windows (service `app.clipforge.desktop`) et n'apparaît ni dans SQLite ni dans les journaux.

## Fichiers générés et à committer

- `src/bindings.ts` est régénéré à chaque `npm run tauri dev` (build de debug) à partir des commandes Rust. La version fournie est écrite à la main pour que `tsc` passe avant le premier lancement. Committez la version régénérée.
- `package-lock.json` et `src-tauri/Cargo.lock` apparaissent au premier `npm install` et au premier build : committez-les.

## Construire l'installeur

```powershell
npm run tauri build -- --bundles nsis
```

L'installeur NSIS est généré dans `src-tauri/target/release/bundle/nsis/`. Pour produire aussi les artefacts de mise à jour signés (ce que fait la CI), ajoutez `--config src-tauri/tauri.release.conf.json` ; la clé privée doit alors être dans la variable d'environnement `TAURI_SIGNING_PRIVATE_KEY`.

## Mises à jour et publication (GitHub Releases)

1. **Dépôt cible, à un seul endroit** : remplacez `VOTRE_COMPTE/clipforge` dans `plugins.updater.endpoints` de `src-tauri/tauri.conf.json`. Le cœur Rust relit cette valeur pour l'updater et pour le manifeste des sidecars.
2. **Clés de l'updater Tauri** :
   ```powershell
   npx tauri signer generate -w "$env:USERPROFILE\.tauri\clipforge.key"
   ```
   Collez la clé publique dans `plugins.updater.pubkey`. Ajoutez le contenu de la clé privée et son mot de passe aux secrets GitHub `TAURI_SIGNING_PRIVATE_KEY` et `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Ne commitez jamais la clé privée. Tant que la clé publique ou le dépôt sont des placeholders, les mises à jour automatiques restent désactivées.
3. **Manifeste distant des sidecars (Ed25519), optionnel** : il permet de corriger une version d'outil (par exemple yt-dlp, que Twitch casse régulièrement) sans republier l'application.
   ```powershell
   npm run manifest:keygen
   ```
   Collez la clé publique affichée dans `SIDECAR_MANIFEST_PUBKEY_HEX` (`src-tauri/src/sidecars/manifest.rs`) et mettez le contenu de la clé privée dans le secret `MANIFEST_SIGNING_KEY`. La CI signe et publie alors `sidecars-manifest.json` et `.sig` avec chaque release. Les releases sont créées en brouillon : publiez-les pour que `/releases/latest/download/` les serve.
4. **Authenticode (plus tard)** : les builds ne sont pas signés, SmartScreen affichera un avertissement. Pour signer, renseignez `bundle.windows.certificateThumbprint` et `bundle.windows.timestampUrl` (ou `bundle.windows.signCommand` pour Azure Trusted Signing) dans `tauri.conf.json`, et importez le certificat dans la CI avant l'étape `tauri-action`.

## Sidecars : ce qu'il faut savoir

- Les outils ne sont pas dans l'installeur : ils sont téléchargés dans `%LOCALAPPDATA%\ClipForge\bin\<outil>\<version>\`, avec reprise de téléchargement, vérification SHA-256 de l'archive puis de l'exécutable, et auto-test. Une installation qui échoue ne touche jamais la version active.
- Les URLs du manifeste pointent des releases précises, pas `latest` : un hash ne peut pas valider un fichier qui change. Relancez le script de synchronisation pour passer à des versions plus récentes.
- **whisper.cpp n'a pas de binaire Windows Vulkan officiel.** Seules les variantes CUDA et CPU sont proposées. L'ancienne variante « vulkan » pointait en réalité vers un build CPU (OpenBLAS).
- Limite connue : seul l'exécutable principal est revérifié à chaque lancement. Les DLL livrées avec whisper.cpp ne sont contrôlées qu'à l'installation, via le hash de l'archive.
- Mode hors ligne : « Importer depuis un dossier » repère par hash les archives du manifeste, avec les mêmes contrôles.

## Structure

```
src/                    Interface React (écrans, composants, stores, i18n)
src-tauri/src/
  ipc/                  Commandes Tauri typées et événements de progression
  sidecars/             Manifeste, téléchargement, vérification, installation
  gemini/               Clé (keyring), models.list ; budget, cache, limiteur à venir
  store/ jobs/          SQLite (migrations) et checkpoints de jobs
  ingest/ signals/ scoring/ asr/ render/   Modules des phases 2 et 3 (vides pour l'instant)
scripts/                Synchronisation du manifeste, signature Ed25519
```

Données locales dans `%LOCALAPPDATA%\ClipForge\` : `bin/`, `cache/`, `projects/`, `models/`, `logs/` et `clipforge.db`.

## Tests

```powershell
npm run build            # tsc puis vite build (le dossier dist est requis par les tests Rust)
cd src-tauri
cargo test
```
