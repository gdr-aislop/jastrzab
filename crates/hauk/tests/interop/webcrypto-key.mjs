// Derives the E2E key exactly as Hauk's web viewer does (frontend/main.js) and prints it as hex.
//   node webcrypto-key.mjs <password> <salt-as-comma-separated-bytes>
const [password, saltArg] = process.argv.slice(2);
const salt = Uint8Array.from(saltArg.split(",").map(Number));
const subtle = globalThis.crypto.subtle;
const base = await subtle.importKey("raw", new TextEncoder("utf-8").encode(password), "PBKDF2", false, ["deriveKey"]);
const key = await subtle.deriveKey(
  { name: "PBKDF2", salt, iterations: 65536, hash: "SHA-1" },
  base,
  { name: "AES-CBC", length: 256 },
  true,
  ["decrypt"],
);
const raw = new Uint8Array(await subtle.exportKey("raw", key));
console.log(Buffer.from(raw).toString("hex"));
