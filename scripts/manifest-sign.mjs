#!/usr/bin/env node
// Génère une paire de clés Ed25519 et signe sidecars-manifest.json.
//
//   node scripts/manifest-sign.mjs keygen
//       Écrit la clé privée (PEM) hors du dépôt et affiche la clé publique à coller dans
//       SIDECAR_MANIFEST_PUBKEY_HEX (src-tauri/src/sidecars/manifest.rs).
//
//   node scripts/manifest-sign.mjs sign <manifest.json>
//       Écrit <manifest.json>.sig (signature hexadécimale de 128 caractères).
//       La clé privée est lue dans MANIFEST_SIGNING_KEY (contenu PEM, pratique en CI)
//       ou dans le fichier indiqué par CLIPFORGE_MANIFEST_KEY
//       (défaut : ~/.clipforge/manifest-ed25519.pem).
import { createPrivateKey, generateKeyPairSync, sign } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

const KEY_PATH =
  process.env.CLIPFORGE_MANIFEST_KEY ?? join(homedir(), ".clipforge", "manifest-ed25519.pem");

function fail(message) {
  console.error(message);
  process.exit(1);
}

function keygen() {
  if (existsSync(KEY_PATH)) {
    fail(`Une clé existe déjà : ${KEY_PATH}\nSupprimez-la explicitement si vous voulez en créer une nouvelle.`);
  }
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  mkdirSync(dirname(KEY_PATH), { recursive: true });
  writeFileSync(KEY_PATH, privateKey.export({ type: "pkcs8", format: "pem" }), { mode: 0o600 });

  // Les 32 derniers octets du DER SPKI sont la clé publique brute.
  const der = publicKey.export({ type: "spki", format: "der" });
  const rawHex = der.subarray(der.length - 32).toString("hex");

  console.log(`Clé privée écrite dans : ${KEY_PATH}`);
  console.log("Ne la commitez jamais. En CI, stockez son contenu dans le secret MANIFEST_SIGNING_KEY.\n");
  console.log("Clé publique (à coller dans SIDECAR_MANIFEST_PUBKEY_HEX) :");
  console.log(rawHex);
}

function signManifest(manifestPath) {
  if (!manifestPath) fail("Usage : node scripts/manifest-sign.mjs sign <manifest.json>");
  if (!existsSync(manifestPath)) fail(`Fichier introuvable : ${manifestPath}`);

  const pem = process.env.MANIFEST_SIGNING_KEY
    ? process.env.MANIFEST_SIGNING_KEY
    : existsSync(KEY_PATH)
      ? readFileSync(KEY_PATH, "utf8")
      : fail(`Aucune clé privée : définissez MANIFEST_SIGNING_KEY ou créez ${KEY_PATH} avec « keygen ».`);

  // La signature porte sur les octets exacts du fichier publié.
  const payload = readFileSync(manifestPath);
  const signature = sign(null, payload, createPrivateKey(pem));

  const sigPath = `${manifestPath}.sig`;
  writeFileSync(sigPath, signature.toString("hex"));
  console.log(`Signature écrite : ${sigPath}`);
}

const [command, arg] = process.argv.slice(2);
if (command === "keygen") keygen();
else if (command === "sign") signManifest(arg);
else fail("Commandes : keygen | sign <manifest.json>");
