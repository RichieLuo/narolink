import {createHash, createPrivateKey, createPublicKey, randomUUID, sign} from "node:crypto";
import {createReadStream} from "node:fs";
import {mkdir, open, readFile, rename, rm, stat, writeFile} from "node:fs/promises";
import {basename, resolve} from "node:path";
import {fileURLToPath} from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const [version, privateKeyFile, ...packages] = process.argv.slice(2);
const stableVersion = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const maxPackageSize = 256 * 1024 * 1024;

async function windowsTarget(file) {
  const handle = await open(file, "r");
  try {
    const dos = Buffer.alloc(64);
    const pe = Buffer.alloc(6);
    const {bytesRead} = await handle.read(dos, 0, dos.length, 0);
    if (bytesRead !== dos.length || dos.toString("ascii", 0, 2) !== "MZ") {
      throw new Error(`${file} is not a Windows executable.`);
    }
    const offset = dos.readUInt32LE(0x3c);
    const header = await handle.read(pe, 0, pe.length, offset);
    if (header.bytesRead !== pe.length || pe.toString("binary", 0, 4) !== "PE\0\0") {
      throw new Error(`${file} has an invalid PE header.`);
    }
    const architecture = new Map([[0x8664, "x86_64"], [0xaa64, "aarch64"], [0x014c, "x86"]])
      .get(pe.readUInt16LE(4));
    if (!architecture) throw new Error(`${file} has an unsupported Windows architecture.`);
    return `windows-${architecture}`;
  } finally {
    await handle.close();
  }
}

async function targets(file) {
  const name = basename(file);
  if (name === "Naro.exe") return [await windowsTarget(file)];
  const prefix = `Naro-${version}-macOS-`;
  if (!name.startsWith(prefix) || !name.endsWith(".zip")) {
    throw new Error(`Unexpected package name for ${version}: ${name}`);
  }
  const arch = name.slice(prefix.length, -4);
  switch (arch) {
    case "arm64": return ["macos-aarch64"];
    case "x86_64": return ["macos-x86_64"];
    case "arm64-x86_64":
    case "x86_64-arm64": return ["macos-aarch64", "macos-x86_64"];
    default: throw new Error(`Unsupported macOS architecture: ${arch}`);
  }
}

async function writeMetadata() {
  if (!stableVersion.test(version) || !privateKeyFile || packages.length === 0) {
    throw new Error("Usage: node scripts/sign-update.mjs <version> <private-key.pem> <package> [package ...]");
  }
  const privateKey = createPrivateKey(await readFile(privateKeyFile));
  const publicKey = createPublicKey(privateKey).export({format: "jwk"});
  const embedded = (await readFile(resolve(root, "assets/update-key.txt"), "utf8")).trim();
  if (publicKey.crv !== "Ed25519" || Buffer.from(publicKey.x, "base64url").toString("base64") !== embedded) {
    throw new Error("The private key does not match the update key bundled in Naro.");
  }

  const assets = {};
  for (const file of packages) {
    const info = await stat(file);
    if (!info.isFile() || info.size < 1 || info.size > maxPackageSize) {
      throw new Error(`Invalid update package size: ${file}`);
    }
    const platforms = await targets(file);
    const hash = createHash("sha256");
    let size = 0;
    for await (const chunk of createReadStream(file)) {
      size += chunk.length;
      if (size > info.size) throw new Error(`The package changed while signing: ${file}`);
      hash.update(chunk);
    }
    if (size !== info.size) throw new Error(`The package changed while signing: ${file}`);
    const asset = {name: basename(file), size, sha256: hash.digest("base64")};
    for (const platform of platforms) {
      if (assets[platform]) throw new Error(`Duplicate update package for ${platform}.`);
      assets[platform] = asset;
    }
  }

  const payload = Buffer.from(JSON.stringify({version, assets}));
  const envelope = JSON.stringify({
    payload: payload.toString("base64"),
    signature: sign(null, payload, privateKey).toString("base64"),
  });
  const dist = resolve(root, "dist");
  await mkdir(dist, {recursive: true});
  const output = resolve(dist, "naro-update-v1.json");
  const temporary = `${output}.${randomUUID()}.tmp`;
  try {
    await writeFile(temporary, `${envelope}\n`, {flag: "wx"});
    await rename(temporary, output);
  } finally {
    await rm(temporary, {force: true});
  }
  console.log(output);
}

try {
  await writeMetadata();
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
