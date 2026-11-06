import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { execFileSync, spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const versionPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const versionFiles = ["package.json", "package-lock.json", "Cargo.toml", "Cargo.lock"];
function numbers(version) {
  const match = versionPattern.exec(version);
  if (!match || !match.slice(1).every(value => Number.isSafeInteger(Number(value)))) throw new Error(`不正なバージョンです: ${version}`);
  return match.slice(1).map(Number);
}
function compare(a, b) {
  const left = numbers(a), right = numbers(b);
  for (let i = 0; i < 3; i++) if (left[i] !== right[i]) return left[i] > right[i] ? 1 : -1;
  return 0;
}
export function chooseVersion(base, versions) {
  if (numbers(base)[0] < 1) throw new Error("初版は1.0.0以降を指定してください");
  const latest = [...versions].sort(compare).at(-1);
  if (!latest || compare(base, latest) > 0) return base;
  const [major, minor, patch] = numbers(latest);
  if (!Number.isSafeInteger(patch + 1)) throw new Error("patch番号が上限を超えました");
  return `${major}.${minor}.${patch + 1}`;
}
export function shouldMakeLatest(version, publishedVersions) {
  return publishedVersions.every(published => compare(version, published) >= 0);
}

function read(root, path) { return readFileSync(join(root, path), "utf8").replaceAll("\r\n", "\n"); }
function json(root, path) { return JSON.parse(read(root, path)); }
function replaceOnce(text, pattern, replacement) {
  if ([...text.matchAll(new RegExp(pattern.source, pattern.flags + "g"))].length !== 1) throw new Error(`バージョンの定義を一意に特定できません: ${pattern}`);
  return text.replace(pattern, replacement);
}
function workspaceSection(cargo) {
  const start = cargo.indexOf("[workspace.package]\n");
  if (start < 0) throw new Error("Cargo.tomlのworkspace.packageが見つかりません");
  const next = cargo.indexOf("\n[", start + 1);
  const end = next < 0 ? cargo.length : next;
  return { start, end, text: cargo.slice(start, end) };
}
function lockPattern(name) { return new RegExp(String.raw`(\[\[package\]\]\nname = "${name}"\nversion = ")[^"]+(")`); }

export function setVersion(root, version) {
  if (numbers(version)[0] < 1) throw new Error("初版は1.0.0以降を指定してください");
  const pkg = json(root, "package.json"), lock = json(root, "package-lock.json");
  if (!lock.packages?.[""]) throw new Error("package-lock.jsonのルートが見つかりません");
  pkg.version = version; lock.version = version; lock.packages[""].version = version;
  const cargo = read(root, "Cargo.toml"), section = workspaceSection(cargo);
  const updatedSection = replaceOnce(section.text, /^version = "[^"]+"$/m, `version = "${version}"`);
  let cargoLock = read(root, "Cargo.lock");
  for (const name of ["simple-vcs", "vcs-core"]) cargoLock = replaceOnce(cargoLock, lockPattern(name), (_match, prefix, suffix) => prefix + version + suffix);
  const updates = [JSON.stringify(pkg, null, 2) + "\n", JSON.stringify(lock, null, 2) + "\n", cargo.slice(0, section.start) + updatedSection + cargo.slice(section.end), cargoLock];
  // 全ファイルの解析を終えてから書き込み、構成不備による部分更新を避ける。
  versionFiles.forEach((path, index) => writeFileSync(join(root, path), updates[index]));
}

export function checkVersions(root) {
  const version = json(root, "package.json").version;
  numbers(version);
  const lock = json(root, "package-lock.json");
  const versions = [lock.version, lock.packages?.[""]?.version, workspaceSection(read(root, "Cargo.toml")).text.match(/^version = "([^"]+)"$/m)?.[1]];
  for (const name of ["simple-vcs", "vcs-core"]) versions.push(read(root, "Cargo.lock").match(lockPattern(name))?.[0].match(/version = "([^"]+)"/)?.[1]);
  if (versions.some(value => value !== version) || json(root, "src-tauri/tauri.conf.json").version !== "../package.json") throw new Error("アプリのバージョンが不一致です。version:setで同期してください");
  return version;
}

function git(root, ...args) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}
function succeeds(root, ...args) {
  const result = spawnSync("git", args, { cwd: root, encoding: "utf8" });
  if (result.error || ![0, 1].includes(result.status)) throw new Error(result.error?.message ?? result.stderr);
  return result.status === 0;
}
function releaseTags(root) {
  const fields = git(root, "for-each-ref", "--format=%(refname:strip=2)%00%(contents)%00", "refs/tags").split("\0");
  const tags = [];
  for (let i = 0; i + 1 < fields.length; i += 2) {
    const name = fields[i].trim(), version = name.slice(1);
    if (!name.startsWith("v") || !versionPattern.test(version)) continue;
    numbers(version);
    const source = fields[i + 1].match(/^Source-Commit: ([a-f0-9]{40})$/m)?.[1];
    tags.push({ name, version, source, commit: git(root, "rev-parse", "--verify", `refs/tags/${name}^{commit}`) });
  }
  return tags.sort((a, b) => compare(b.version, a.version));
}

export function prepareRelease(root, source) {
  if (!/^[a-f0-9]{40}$/.test(source ?? "") || git(root, "rev-parse", "HEAD") !== source) throw new Error("checkoutしたHEADとリリース対象SHAが一致しません");
  if (git(root, "status", "--porcelain", "--untracked-files=no")) throw new Error("未保存の変更があるため採番を停止します");
  const base = checkVersions(root), tags = releaseTags(root);
  const existing = tags.find(tag => tag.source === source);
  if (existing) {
    if (!succeeds(root, "merge-base", "--is-ancestor", source, existing.commit) || JSON.parse(git(root, "show", `${existing.name}:package.json`)).version !== existing.version) throw new Error("既存タグとソースの対応が不正です");
    return { version: existing.version, tag: existing.name, commit: existing.commit, skip: false };
  }
  const latest = tags[0];
  if (latest?.source) {
    if (succeeds(root, "merge-base", "--is-ancestor", source, latest.source)) return { skip: true };
    if (!succeeds(root, "merge-base", "--is-ancestor", latest.source, source)) throw new Error("mainの履歴が以前のReleaseとつながっていません。履歴を確認してください");
  }
  const version = chooseVersion(base, tags.map(tag => tag.version)), tag = `v${version}`;
  git(root, "checkout", "--detach", source);
  setVersion(root, version);
  checkVersions(root);
  const identity = ["-c", "user.name=github-actions[bot]", "-c", "user.email=41898282+github-actions[bot]@users.noreply.github.com", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"];
  if (!succeeds(root, "diff", "--quiet", "--", ...versionFiles)) {
    git(root, "add", "--", ...versionFiles);
    git(root, ...identity, "commit", "-m", `chore(release): ${tag}`, "--", ...versionFiles);
  }
  git(root, ...identity, "tag", "-a", tag, "-m", `Simple VCS ${tag}\n\nSource-Commit: ${source}`);
  return { version, tag, commit: git(root, "rev-parse", "HEAD"), skip: false };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const command = process.argv[2];
  if (command === "set") setVersion(root, process.argv[3]);
  else if (command === "check") console.log(`Version: ${checkVersions(root)}`);
  else if (command === "prepare") {
    const result = prepareRelease(root, process.env.RELEASE_SOURCE_SHA ?? process.env.GITHUB_SHA);
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(result).map(([key, value]) => `${key}=${value}\n`).join(""));
    console.log(JSON.stringify(result));
  } else throw new Error("Usage: release.mjs set <version> | check | prepare");
}
