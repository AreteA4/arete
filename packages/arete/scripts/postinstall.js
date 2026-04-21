#!/usr/bin/env node

const https = require("https");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const pkg = require("../package.json");
const version = pkg.version;

const PLATFORMS = {
  "darwin-arm64": "a4-darwin-arm64",
  "darwin-x64": "a4-darwin-x64",
  "linux-x64": "a4-linux-x64",
  "linux-arm64": "a4-linux-arm64",
  "win32-x64": "a4-win32-x64.exe",
};

const platform = process.platform;
const arch = process.arch;
const key = `${platform}-${arch}`;

const binaryName = PLATFORMS[key];
if (!binaryName) {
  console.warn(
    `Arete CLI does not have a prebuilt binary for ${key}.\n` +
    "You can build from source: cargo install a4-cli"
  );
  process.exit(0);
}

const binDir = path.join(__dirname, "..", "bin");
const binPath = path.join(binDir, platform === "win32" ? "a4.exe" : "a4");

if (fs.existsSync(binPath)) {
  process.exit(0);
}

const releaseUrl = `https://github.com/AreteA4/arete/releases/download/a4-cli-v${version}`;
const url = `${releaseUrl}/${binaryName}`;
const checksumUrl = `${releaseUrl}/checksums.txt`;
const MAX_REDIRECTS = 5;

console.log(`Downloading Arete CLI v${version} for ${key}...`);

function removeIfExists(filePath) {
  try {
    fs.unlinkSync(filePath);
  } catch (err) {
    if (err.code !== "ENOENT") {
      throw err;
    }
  }
}

function getResponse(url, depth = 0) {
  if (depth > MAX_REDIRECTS) {
    return Promise.reject(new Error("Too many redirects"));
  }

  return new Promise((resolve, reject) => {
    https
      .get(url, (response) => {
        if (response.statusCode === 302 || response.statusCode === 301) {
          if (!response.headers.location) {
            response.resume();
            reject(new Error("Redirect missing location header"));
            return;
          }

          response.resume();
          resolve(
            getResponse(
              new URL(response.headers.location, url).toString(),
              depth + 1
            )
          );
          return;
        }

        resolve(response);
      })
      .on("error", reject);
  });
}

async function download(url, dest) {
  const response = await getResponse(url);

  if (response.statusCode !== 200) {
    response.resume();
    removeIfExists(dest);
    throw new Error(`Download failed: HTTP ${response.statusCode}`);
  }

  const total = parseInt(response.headers["content-length"], 10);
  let downloaded = 0;

  await new Promise((resolve, reject) => {
    const file = fs.createWriteStream(dest, { mode: 0o755 });

    response.on("data", (chunk) => {
      downloaded += chunk.length;
      if (total && process.stdout.isTTY) {
        const pct = Math.round((downloaded / total) * 100);
        process.stdout.write(`\rDownloading... ${pct}%`);
      }
    });

    response.on("error", (err) => {
      file.destroy();
      removeIfExists(dest);
      reject(err);
    });

    file.on("error", (err) => {
      response.destroy();
      removeIfExists(dest);
      reject(err);
    });

    file.on("finish", () => {
      file.close(() => {
        if (process.stdout.isTTY) {
          process.stdout.write("\n");
        }
        resolve();
      });
    });

    response.pipe(file);
  });
}

async function fetchText(url) {
  const response = await getResponse(url);

  if (response.statusCode !== 200) {
    response.resume();
    throw new Error(`Checksum download failed: HTTP ${response.statusCode}`);
  }

  return new Promise((resolve, reject) => {
    let body = "";
    response.setEncoding("utf8");
    response.on("data", (chunk) => {
      body += chunk;
    });
    response.on("end", () => resolve(body));
    response.on("error", reject);
  });
}

function sha256File(filePath) {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash("sha256");
    const stream = fs.createReadStream(filePath);

    stream.on("error", reject);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("end", () => resolve(hash.digest("hex")));
  });
}

async function verifyChecksum(checksumUrl, fileName, filePath) {
  const checksums = await fetchText(checksumUrl);
  const expected = checksums
    .split(/\r?\n/)
    .map((line) => line.match(/^([a-fA-F0-9]{64})\s+\*?(.+)$/))
    .find((match) => match && match[2] === fileName);

  if (!expected) {
    throw new Error(`Missing checksum for ${fileName}`);
  }

  const actual = await sha256File(filePath);
  if (actual !== expected[1].toLowerCase()) {
    throw new Error(`Checksum mismatch for ${fileName}`);
  }
}

async function main() {
  try {
    if (!fs.existsSync(binDir)) {
      fs.mkdirSync(binDir, { recursive: true });
    }
    
    await download(url, binPath);

    try {
      await verifyChecksum(checksumUrl, binaryName, binPath);
    } catch (err) {
      removeIfExists(binPath);
      throw err;
    }

    if (platform !== "win32") {
      fs.chmodSync(binPath, 0o755);
    }

    console.log("Arete CLI installed successfully.");
  } catch (err) {
    console.error(`\nFailed to download Arete CLI: ${err.message}`);
    console.error(
      "\nYou can install manually via Cargo:\n" +
      "  cargo install a4-cli\n" +
      "\nOr download directly from:\n" +
      `  ${url}`
    );
    process.exit(0);
  }
}

main();
