import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// Creates a new project-local fixture. Never consumes a Panel .env or user database.
process.umask(0o077);
const here = path.dirname(fileURLToPath(import.meta.url));
const orbit = path.resolve(here, "../../..");
const args = process.argv.slice(2);
function option(name) {
  const index = args.indexOf(name);
  if (index < 0 || !args[index + 1])
    throw new Error("missing_acceptance_option");
  return args[index + 1];
}
const panelRepo = fs.realpathSync(option("--panel-repo"));
const cachedPanel = fs.realpathSync(option("--cached-panel"));
const panelRef = option("--panel-ref");
const phpBase = option("--php-base");
if (
  !/^[a-f0-9]{40}$/.test(panelRef) ||
  !/^[a-z0-9./_-]+@sha256:[a-f0-9]{64}$/.test(phpBase)
) {
  throw new Error("unpinned_acceptance_source");
}
const port = Number(option("--port"));
const wrongSanPort = Number(option("--wrong-san-port"));
if (
  ![port, wrongSanPort].every(
    (v) => Number.isInteger(v) && v >= 1024 && v <= 65535,
  ) ||
  port === wrongSanPort
) {
  throw new Error("invalid_acceptance_ports");
}
const id = `${new Date()
  .toISOString()
  .replace(/[-:TZ.]/g, "")
  .slice(0, 14)}-${crypto.randomBytes(3).toString("hex")}`;
const run = path.join(orbit, ".cache/panel-acceptance", id);
fs.mkdirSync(run, { recursive: true, mode: 0o700 });
const source = path.join(run, "source");
const certs = path.join(run, "certs");
for (const directory of [
  source,
  certs,
  path.join(run, "empty-env"),
  path.join(run, "tmp"),
  path.join(run, "composer"),
]) {
  fs.mkdirSync(directory, { mode: 0o700 });
}
fs.chmodSync(source, 0o755); // Container workers can traverse only this mounted snapshot.
const digest = (bytes) =>
  crypto.createHash("sha256").update(bytes).digest("hex");
function command(executable, values, options = {}) {
  const result = spawnSync(executable, values, {
    cwd: source,
    encoding: "utf8",
    maxBuffer: 8 * 1024 * 1024,
    ...options,
  });
  if (result.status !== 0) throw new Error("acceptance_preparation_failed");
  return result.stdout;
}
const gitEnv = {
  ...process.env,
  DEVELOPER_DIR: "/Library/Developer/CommandLineTools",
};
const archive = path.join(run, "panel-source.tar");
const archiveFd = fs.openSync(archive, "wx", 0o600);
try {
  command("/usr/bin/git", ["archive", "--format=tar", panelRef], {
    cwd: panelRepo,
    env: gitEnv,
    stdio: ["ignore", archiveFd, "pipe"],
  });
} finally {
  fs.closeSync(archiveFd);
}
command("/usr/bin/tar", ["-xf", archive, "-C", source]);
for (const lock of ["composer.lock", "package-lock.json"]) {
  if (
    !fs
      .readFileSync(path.join(source, lock))
      .equals(fs.readFileSync(path.join(cachedPanel, lock)))
  ) {
    throw new Error("cached_dependency_lock_mismatch");
  }
}
for (const directory of ["vendor", "node_modules"]) {
  const from = path.join(cachedPanel, directory);
  if (fs.lstatSync(from).isSymbolicLink())
    throw new Error("external_dependency_symlink");
  fs.cpSync(from, path.join(source, directory), {
    recursive: true,
    force: false,
    errorOnExist: true,
    verbatimSymlinks: true,
  });
}
function internalSnapshotLinks(directory) {
  for (const item of fs.readdirSync(directory, { withFileTypes: true })) {
    const filename = path.join(directory, item.name);
    if (item.isSymbolicLink()) {
      if (!fs.realpathSync(filename).startsWith(`${source}${path.sep}`))
        throw new Error("external_snapshot_symlink");
    } else if (item.isDirectory()) {
      internalSnapshotLinks(filename);
    }
  }
}
internalSnapshotLinks(source); // Before executing cached Composer/Vite code.
const appKey = `base64:${crypto.randomBytes(32).toString("base64")}`;
const buildEnv = {
  PATH: process.env.PATH,
  TMPDIR: path.join(run, "tmp"),
  APP_ENV: "local",
  APP_DEBUG: "false",
  APP_KEY: appKey,
  DB_CONNECTION: "sqlite",
  DB_DATABASE: path.join(run, "build.sqlite"),
  SESSION_DRIVER: "array",
  CACHE_STORE: "array",
  QUEUE_CONNECTION: "sync",
  LOG_CHANNEL: "stderr",
  COMPOSER_HOME: path.join(run, "composer"),
  COMPOSER_CACHE_DIR: path.join(run, "composer"),
  COMPOSER_DISABLE_NETWORK: "1",
};
fs.writeFileSync(buildEnv.DB_DATABASE, "");
for (const [label, executable, values] of [
  [
    "composer",
    "composer",
    [
      "install",
      "--no-dev",
      "--prefer-dist",
      "--no-interaction",
      "--no-scripts",
      "--no-progress",
    ],
  ],
  ["platform", "composer", ["check-platform-reqs", "--no-dev"]],
  [
    "assets",
    process.execPath,
    [
      "--input-type=module",
      "-e",
      "const {build}=await import('vite'); await build({configFile:process.argv[1],envDir:process.argv[2]});",
      path.join(source, "vite.config.ts"),
      path.join(run, "empty-env"),
    ],
  ],
]) {
  const log = fs.openSync(path.join(run, `prepare-${label}.log`), "wx", 0o600);
  try {
    command(executable, values, { env: buildEnv, stdio: ["ignore", log, log] });
  } finally {
    fs.closeSync(log);
  }
}
// The snapshot contains tracked source and dependency/build artifacts only.
// Its host UID differs from the non-root container UID; secrets stay outside it.
function readableSnapshot(directory) {
  for (const item of fs.readdirSync(directory, { withFileTypes: true })) {
    const filename = path.join(directory, item.name);
    const stat = fs.lstatSync(filename);
    if (stat.isSymbolicLink()) {
      if (!fs.realpathSync(filename).startsWith(`${source}${path.sep}`))
        throw new Error("external_snapshot_symlink");
    } else if (stat.isDirectory()) {
      readableSnapshot(filename);
      fs.chmodSync(filename, 0o755);
    } else if (stat.isFile()) {
      fs.chmodSync(filename, stat.mode & 0o111 ? 0o755 : 0o644);
    } else {
      throw new Error("unsupported_snapshot_entry");
    }
  }
}
readableSnapshot(source);
function openssl(values) {
  command("openssl", values, {
    cwd: certs,
    stdio: ["ignore", "ignore", "pipe"],
  });
}
for (const name of ["ca", "bad-ca"]) {
  openssl([
    "req",
    "-x509",
    "-newkey",
    "ec",
    "-pkeyopt",
    "ec_paramgen_curve:P-256",
    "-nodes",
    "-days",
    "2",
    "-subj",
    `/CN=Orbit isolated ${name}`,
    "-addext",
    "basicConstraints=critical,CA:TRUE",
    "-addext",
    "keyUsage=critical,keyCertSign,cRLSign",
    "-keyout",
    `${name}.key`,
    "-out",
    `${name}.pem`,
  ]);
}
for (const [name, hostname] of [
  ["server", "orbit-e2e.test"],
  ["wrong-san", "wrong-orbit-e2e.test"],
]) {
  openssl([
    "req",
    "-new",
    "-newkey",
    "ec",
    "-pkeyopt",
    "ec_paramgen_curve:P-256",
    "-nodes",
    "-subj",
    `/CN=${hostname}`,
    "-keyout",
    `${name}.key`,
    "-out",
    `${name}.csr`,
  ]);
  fs.writeFileSync(
    path.join(certs, `${name}.ext`),
    `basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:${hostname}\n`,
  );
  openssl([
    "x509",
    "-req",
    "-in",
    `${name}.csr`,
    "-CA",
    "ca.pem",
    "-CAkey",
    "ca.key",
    "-CAcreateserial",
    "-days",
    "2",
    "-extfile",
    `${name}.ext`,
    "-out",
    `${name}.pem`,
  ]);
}
const dbName = `orivelle_orbit_acceptance_${id.replaceAll("-", "_")}`;
const dbPassword = crypto.randomBytes(24).toString("base64url");
const syntheticPassword = crypto.randomBytes(24).toString("base64url");
function dotenv(filename, values) {
  fs.writeFileSync(
    path.join(run, filename),
    Object.entries(values)
      .map(([key, value]) => `${key}=${JSON.stringify(String(value))}`)
      .join("\n") + "\n",
    { mode: 0o600 },
  );
}
dotenv("mysql.env", {
  MYSQL_ROOT_PASSWORD: crypto.randomBytes(24).toString("base64url"),
  MYSQL_DATABASE: dbName,
  MYSQL_USER: "orbit_acceptance",
  MYSQL_PASSWORD: dbPassword,
});
dotenv("panel.env", {
  APP_NAME: "Orbit isolated Panel",
  APP_ENV: "local",
  APP_DEBUG: "false",
  APP_KEY: appKey,
  APP_URL: `https://orbit-e2e.test:${port}`,
  APP_LOCALE: "zh_CN",
  APP_MAINTENANCE_DRIVER: "file",
  DB_CONNECTION: "mysql",
  DB_HOST: "mysql",
  DB_PORT: "3306",
  DB_DATABASE: dbName,
  DB_USERNAME: "orbit_acceptance",
  DB_PASSWORD: dbPassword,
  SESSION_DRIVER: "database",
  SESSION_SECURE_COOKIE: "true",
  SESSION_SAME_SITE: "lax",
  CACHE_STORE: "database",
  QUEUE_CONNECTION: "database",
  LOG_CHANNEL: "stderr",
  LOG_LEVEL: "warning",
  MAIL_MAILER: "log",
});
const project = `orbit-panel-01a132f1-${id}`;
dotenv("compose.env", {
  ORBIT_ACCEPTANCE_PROJECT: project,
  ORBIT_ACCEPTANCE_PHP_IMAGE: `orbit-panel-acceptance:${id}`,
  ORBIT_ACCEPTANCE_PHP_BASE: phpBase,
  ORBIT_ACCEPTANCE_PANEL_ENV: path.join(run, "panel.env"),
  ORBIT_ACCEPTANCE_MYSQL_ENV: path.join(run, "mysql.env"),
  ORBIT_ACCEPTANCE_SOURCE: source,
  ORBIT_ACCEPTANCE_CERTS: certs,
  ORBIT_ACCEPTANCE_PORT: port,
  ORBIT_ACCEPTANCE_WRONG_SAN_PORT: wrongSanPort,
});
const input = {
  port,
  wrong_san_port: wrongSanPort,
  ca_file: path.join(certs, "ca.pem"),
  bad_ca_file: path.join(certs, "bad-ca.pem"),
  node: process.execPath,
  web_driver: path.join(here, "web-driver.mjs"),
  email: "orbit-e2e@example.test",
  password: syntheticPassword,
  expected_account_id: null,
  expected_node_id: null,
  expected_model_ids: ["orbit-synthetic-text"],
  denied_model_ids: ["orbit-synthetic-denied"],
};
fs.writeFileSync(
  path.join(run, "native-input.json"),
  JSON.stringify(input, null, 2) + "\n",
  { mode: 0o600 },
);
fs.writeFileSync(
  path.join(run, "source-manifest.json"),
  JSON.stringify(
    {
      panel_ref: panelRef,
      source_archive_sha256: digest(fs.readFileSync(archive)),
      composer_lock_sha256: digest(
        fs.readFileSync(path.join(source, "composer.lock")),
      ),
      npm_lock_sha256: digest(
        fs.readFileSync(path.join(source, "package-lock.json")),
      ),
      assets_manifest_sha256: digest(
        fs.readFileSync(path.join(source, "public/build/manifest.json")),
      ),
      php_base: phpBase,
      database: dbName,
      project,
      boundary:
        "Panel authorization/API component; no Edge/upstream/Desktop GUI acceptance",
    },
    null,
    2,
  ) + "\n",
);
console.log(
  JSON.stringify({
    ok: true,
    run,
    project,
    native_input: path.join(run, "native-input.json"),
  }),
);
