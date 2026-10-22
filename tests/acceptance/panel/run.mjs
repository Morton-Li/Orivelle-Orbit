import fs from "node:fs";
import path from "node:path";
import https from "node:https";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

process.umask(0o077);
const here = path.dirname(fileURLToPath(import.meta.url));
const orbit = path.resolve(here, "../../..");
const [action, directory] = process.argv.slice(2);
if (!["setup", "cleanup"].includes(action) || !directory)
  throw new Error("invalid_acceptance_action");
const run = fs.realpathSync(directory);
if (
  !run.startsWith(
    `${fs.realpathSync(path.join(orbit, ".cache/panel-acceptance"))}${path.sep}`,
  )
) {
  throw new Error("external_acceptance_directory");
}
const manifest = JSON.parse(
  fs.readFileSync(path.join(run, "source-manifest.json")),
);
const inputFile = path.join(run, "native-input.json");
const input = JSON.parse(fs.readFileSync(inputFile));
if (!/^orbit-panel-01a132f1-[0-9]{14}-[a-f0-9]{6}$/.test(manifest.project))
  throw new Error("invalid_acceptance_project");
const docker = "/usr/local/bin/docker";
let dockerEnv = { PATH: process.env.PATH, TMPDIR: path.join(run, "tmp") };
const base = [
  "compose",
  "--env-file",
  path.join(run, "compose.env"),
  "--project-name",
  manifest.project,
  "--file",
  path.join(here, "compose.yml"),
];
const attempt = `${action}-${Date.now()}`;
const report = {
  action,
  project: manifest.project,
  panel_ref: manifest.panel_ref,
  stages: [],
  passed: false,
};
function command(stage, values, stdin) {
  const logPath = path.join(
    run,
    `${attempt}-${report.stages.length + 1}-${stage}.log`,
  );
  const log = fs.openSync(logPath, "wx", 0o600);
  let result;
  try {
    result = spawnSync(docker, values, {
      input: stdin,
      env: dockerEnv,
      encoding: "utf8",
      maxBuffer: 4 * 1024 * 1024,
      stdio: ["pipe", "pipe", log],
    });
    fs.writeSync(log, result.stdout ?? "");
  } finally {
    fs.closeSync(log);
  }
  report.stages.push({
    stage,
    exit_code: result.status,
    log: path.basename(logPath),
  });
  if (result.status !== 0) throw new Error(`acceptance_stage_failed:${stage}`);
  console.log(JSON.stringify({ stage, passed: true }));
  return result.stdout;
}
function compose(stage, values, stdin) {
  return command(stage, [...base, ...values], stdin);
}
function verifyOwnership(requireEmpty) {
  for (const [kind, values, name] of [
    ["containers", ["ps", "--all"], "{{.ID}}"],
    ["networks", ["network", "ls"], "{{.Name}}"],
    ["volumes", ["volume", "ls"], "{{.Name}}"],
  ]) {
    const output = command(`ownership_${kind}`, [
      ...values,
      "--filter",
      `label=com.docker.compose.project=${manifest.project}`,
      "--format",
      `${name} {{.Label "codex.task"}} {{.Label "codex.scope"}}`,
    ]);
    const lines = output.trim().split("\n").filter(Boolean);
    if (
      (requireEmpty && lines.length) ||
      lines.some((line) => {
        const [id, task, scope, extra] = line.split(" ");
        return (
          extra ||
          task !== "orbit-desktop-01a132f1" ||
          scope !== "panel-acceptance" ||
          (kind === "containers"
            ? !/^[a-f0-9]+$/.test(id)
            : !id.startsWith(`${manifest.project}_`))
        );
      })
    )
      throw new Error("acceptance_resource_ownership_conflict");
  }
}
async function ready() {
  const ca = fs.readFileSync(input.ca_file);
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    const accepted = await new Promise((resolve) => {
      const request = https.get(
        {
          hostname: "orbit-e2e.test",
          servername: "orbit-e2e.test",
          port: input.port,
          path: "/login",
          ca,
          rejectUnauthorized: true,
          agent: false,
          family: 4,
          autoSelectFamily: false,
          lookup: (_hostname, _options, callback) =>
            callback(null, "127.0.0.1", 4),
          timeout: 2000,
        },
        (response) => {
          response.resume();
          resolve(response.statusCode === 200);
        },
      );
      request.on("timeout", () => request.destroy());
      request.on("error", () => resolve(false));
    });
    if (accepted) return;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error("acceptance_tls_runtime_not_ready");
}
try {
  if (
    command("context", [
      "--context",
      "desktop-linux",
      "context",
      "show",
    ]).trim() !== "desktop-linux"
  )
    throw new Error("nonlocal_docker_context");
  const socket = JSON.parse(
    command("socket", [
      "--context",
      "desktop-linux",
      "context",
      "inspect",
      "desktop-linux",
      "--format",
      "{{json .Endpoints.docker.Host}}",
    ]),
  );
  if (
    typeof socket !== "string" ||
    !socket.startsWith("unix:///") ||
    !fs.statSync(socket.slice("unix://".length)).isSocket()
  ) {
    throw new Error("acceptance_nonlocal_docker_socket");
  }
  const config = path.join(run, "docker-config");
  fs.mkdirSync(config, { recursive: true, mode: 0o700 });
  fs.writeFileSync(
    path.join(config, "config.json"),
    JSON.stringify({
      cliPluginsExtraDirs: [
        "/Applications/Docker.app/Contents/Resources/cli-plugins",
      ],
    }),
    { mode: 0o600 },
  );
  dockerEnv = {
    PATH: process.env.PATH,
    TMPDIR: path.join(run, "tmp"),
    DOCKER_CONFIG: config,
    DOCKER_HOST: socket,
    BUILDX_CONFIG: path.join(config, "buildx"),
    XDG_CACHE_HOME: path.join(config, "cache"),
  }; // No inherited COMPOSE_*, ORBIT_ACCEPTANCE_*, proxy or Docker overrides.
  verifyOwnership(action === "setup");
  compose("config", ["config", "--quiet"]); // Do not print rendered secret environment.
  if (action === "cleanup") {
    compose("down", ["down", "--volumes"]);
    verifyOwnership(true);
  } else {
    if (fs.existsSync(path.join(run, "runtime-setup.json")))
      throw new Error("acceptance_already_initialized");
    compose("build", ["build", "panel"]);
    compose("mysql", ["up", "--detach", "--wait", "mysql"]);
    compose("volumes", ["run", "-T", "--rm", "--no-deps", "prepare"]);
    compose("migrate", [
      "run",
      "-T",
      "--rm",
      "--no-deps",
      "panel",
      "php",
      "artisan",
      "migrate",
      "--force",
      "--no-interaction",
    ]);
    const seed = JSON.parse(
      compose(
        "seed",
        [
          "run",
          "-T",
          "--rm",
          "--no-deps",
          "--volume",
          `${path.join(here, "seed.php")}:/fixture/seed.php:ro`,
          "panel",
          "php",
          "/fixture/seed.php",
        ],
        JSON.stringify({
          panelRoot: "/app",
          database: manifest.database,
          email: input.email,
          password: input.password,
        }),
      ),
    );
    if (
      seed.ok !== true ||
      seed.databaseVerified !== true ||
      seed.nodeAvailable !== false ||
      seed.nodeState !== "unavailable"
    ) {
      throw new Error("invalid_acceptance_seed_receipt");
    }
    input.expected_account_id = String(seed.accountId);
    input.expected_node_id = seed.nodeId;
    input.expected_model_ids = seed.modelIds;
    input.denied_model_ids = seed.deniedModelIds;
    fs.writeFileSync(inputFile, JSON.stringify(input, null, 2) + "\n", {
      mode: 0o600,
    });
    compose("services", [
      "up",
      "--detach",
      "--no-build",
      "panel",
      "worker",
      "scheduler",
      "ingress",
    ]);
    await ready();
    report.stages.push({ stage: "strict_tls_login_page", passed: true });
    fs.writeFileSync(
      path.join(run, "runtime-setup.json"),
      JSON.stringify(
        {
          project: manifest.project,
          panel_ref: manifest.panel_ref,
          seed,
          ready: true,
          boundary:
            "Real isolated Panel HTTP runtime; node offline; no Edge, upstream or Desktop GUI evidence",
        },
        null,
        2,
      ) + "\n",
    );
  }
  report.passed = true;
} catch (error) {
  // Only fixed stage identifiers are surfaced. Child logs remain project-local.
  report.error = /^acceptance_[a-z_]+(?::[a-z_]+)?$/.test(error.message)
    ? error.message
    : "acceptance_failed";
  process.exitCode = 1;
} finally {
  fs.writeFileSync(
    path.join(run, `${attempt}.json`),
    JSON.stringify(report, null, 2) + "\n",
  );
  console.log(
    JSON.stringify({
      action,
      passed: report.passed,
      error: report.error ?? null,
    }),
  );
}
