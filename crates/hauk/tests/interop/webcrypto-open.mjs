// Reads the JSON printed by `cargo run -p hauk --example seal_post` on stdin and decrypts every
// field the way Hauk's web viewer does. Exits non-zero unless the plaintext is what was sealed.
import { webcrypto } from "node:crypto";
import { readFileSync } from "node:fs";
const post = JSON.parse(readFileSync(0, "utf8"));
const byteArray = (b64) => Uint8Array.from(Buffer.from(b64, "base64"));
// Not globalThis.crypto: Node 18 (Debian bookworm's) has no global WebCrypto without a flag.
const subtle = webcrypto.subtle;
const base = await subtle.importKey("raw", new TextEncoder("utf-8").encode(post.password), "PBKDF2", false, ["deriveKey"]);
const key = await subtle.deriveKey(
  { name: "PBKDF2", salt: byteArray(post.salt), iterations: 65536, hash: "SHA-1" },
  base, { name: "AES-CBC", length: 256 }, false, ["decrypt"]);
const algo = { name: "AES-CBC", iv: byteArray(post.iv) };
const open = async (field) => new TextDecoder().decode(await subtle.decrypt(algo, key, byteArray(post[field])));
const got = Object.fromEntries(await Promise.all(["lat", "lon", "prv", "time", "spd", "acc"].map(async (f) => [f, await open(f)])));
const want = { lat: "52.5200066", lon: "13.404954", prv: "0", time: "1700000000.25", spd: "1.5", acc: "4" };
console.log(JSON.stringify(got));
for (const [k, v] of Object.entries(want)) {
  if (got[k] !== v) { console.error(`mismatch in ${k}: got ${got[k]}, want ${v}`); process.exit(1); }
}
console.log("viewer-side decryption OK");
