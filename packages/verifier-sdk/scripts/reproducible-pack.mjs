// Reproducible npm tarballs + SRI hashes for CDN consumers.
//
//   node scripts/reproducible-pack.mjs --out <dir> [--sri <file>]
//
// `npm pack` embeds the current mtime in every tar entry and lets the
// filesystem decide entry order, so two builds of the same tree produce
// different tarballs. This script packs the package, then rewrites the
// tarball deterministically:
//
//   - entries sorted by path (npm's own order: package.json first, then
//     lexicographic), directories before their contents;
//   - every mtime fixed to SOURCE_DATE_EPOCH (default 1985-10-26T08:15:00Z,
//     the npm/tar convention), uid/gid 0, uname/gname empty, mode 0644/0755;
//   - gzip header mtime 0 and no embedded file name.
//
// The resulting tarball is byte-identical across machines and runs, so CI can
// build twice and compare hashes. `--sri` writes the Subresource Integrity
// hashes (sha512, plus sha384/sha256) of the tarball for release notes.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { gunzipSync, gzipSync } from "node:zlib";

const pkgDir = fileURLToPath(new URL("..", import.meta.url));
const FIXED_MTIME = Number(process.env.SOURCE_DATE_EPOCH || 499162500); // 1985-10-26T08:15:00Z

const args = process.argv.slice(2);
const arg = (name) => {
  const i = args.indexOf(name);
  return i === -1 ? undefined : args[i + 1];
};
const outDir = arg("--out");
const sriFile = arg("--sri");
if (!outDir) {
  console.error("usage: node scripts/reproducible-pack.mjs --out <dir> [--sri <file>]");
  process.exit(2);
}

/** Parse a ustar/pax tar buffer into entries (name, mode, type, body). */
function parseTar(buf) {
  const entries = [];
  let off = 0;
  let paxPath = null;
  while (off + 512 <= buf.length) {
    const header = buf.subarray(off, off + 512);
    if (header.every((b) => b === 0)) break;
    const str = (start, len) => header.subarray(start, start + len).toString("utf8").replace(/\0.*$/s, "");
    const size = parseInt(str(124, 12).trim() || "0", 8);
    const type = String.fromCharCode(header[156] || 48);
    const body = buf.subarray(off + 512, off + 512 + size);
    off += 512 + Math.ceil(size / 512) * 512;
    if (type === "x") {
      // pax extended header: "len key=value\n"
      const text = body.toString("utf8");
      const m = /(?:^|\n)\d+ path=([^\n]*)\n/.exec(text);
      if (m) paxPath = m[1];
      continue;
    }
    const name = paxPath ?? str(0, 100);
    paxPath = null;
    entries.push({ name, mode: parseInt(str(100, 8).trim() || "644", 8), type, body });
  }
  return entries;
}

function octal(value, len) {
  return value.toString(8).padStart(len - 1, "0") + "\0";
}

function header(entry) {
  const h = Buffer.alloc(512);
  const name = Buffer.from(entry.name, "utf8");
  if (name.length > 100) throw new Error(`path too long for ustar: ${entry.name}`);
  name.copy(h, 0);
  h.write(octal(entry.mode, 8), 100, "ascii");
  h.write(octal(0, 8), 108, "ascii"); // uid
  h.write(octal(0, 8), 116, "ascii"); // gid
  h.write(octal(entry.body.length, 12), 124, "ascii");
  h.write(octal(FIXED_MTIME, 12), 136, "ascii");
  h.write("        ", 148, "ascii"); // checksum placeholder
  h.write(entry.type, 156, "ascii");
  h.write("ustar\0" + "00", 257, "ascii");
  let sum = 0;
  for (const b of h) sum += b;
  h.write(octal(sum, 8), 148, "ascii");
  return h;
}

function buildTar(entries) {
  const sorted = [...entries].sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  const parts = [];
  for (const e of sorted) {
    parts.push(header(e), e.body, Buffer.alloc((512 - (e.body.length % 512)) % 512));
  }
  parts.push(Buffer.alloc(1024));
  return Buffer.concat(parts);
}

const scratch = mkdtempSync(join(tmpdir(), "lafiya-pack-"));
try {
  const packed = execFileSync("npm", ["pack", "--json", "--pack-destination", scratch], {
    cwd: pkgDir,
    stdio: ["ignore", "pipe", "inherit"],
  }).toString();
  const [{ filename }] = JSON.parse(packed);
  const raw = gunzipSync(readFileSync(join(scratch, filename)));
  const tar = buildTar(parseTar(raw));
  const gz = gzipSync(tar, { level: 9, mtime: 0 });
  writeFileSync(join(outDir, filename), gz);

  const digest = (alg) => `${alg}-${createHash(alg).update(gz).digest("base64")}`;
  const sri = { file: filename, size: gz.length, sha512: digest("sha512"), sha384: digest("sha384"), sha256: digest("sha256") };
  if (sriFile) writeFileSync(sriFile, JSON.stringify(sri, null, 2) + "\n");
  process.stdout.write(JSON.stringify(sri) + "\n");
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
