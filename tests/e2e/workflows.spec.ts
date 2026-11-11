import { expect, test, type Page } from "@playwright/test";
import manifest from "../../package.json" with { type: "json" };
import { bootstrap, task } from "../fixtures";
import {
  calls,
  fixture,
  pushState,
  type FixtureControl,
} from "./native-fixture";

async function choose(page: Page, id: string, label: string) {
  await page.locator(`#${id}`).click();
  await page.getByRole("option", { name: label, exact: true }).click();
}
async function chooseClaudeRoles(page: Page) {
  for (const role of ["primary", "fallback", "auxiliary"]) {
    await choose(page, `claude-${role}`, "synthetic-text");
  }
}

test("unconnected UI, navigation, keyboard, modal focus and narrow container", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(
    "让 AI 工具接入 Orivelle",
  );
  await expect(page.getByText("待检测", { exact: true })).toHaveCount(2);
  await page.screenshot({
    path: ".cache/evidence/home-unconnected.png",
    fullPage: true,
  });
  const login = page
    .getByRole("button", { name: "连接账户", exact: true })
    .last();
  await login.click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(login).toBeFocused();
  await page.keyboard.press("Control+3");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(
    "管理桌面接入",
  );
  await page.setViewportSize({ width: 390, height: 844 });
  for (const name of ["首页", "图片工作台", "接入", "设置"]) {
    await page
      .getByRole("navigation")
      .getByRole("link", { name, exact: true })
      .click();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
  }
  await page.screenshot({
    path: ".cache/evidence/settings-narrow.png",
    fullPage: true,
  });
});

test("real capability names populate client models and image edit options", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/access");
  await expect(page.locator("#model-codex")).toHaveCount(0);
  await page.locator("#claude-primary").click();
  await expect(
    page.getByRole("option", { name: "synthetic-text", exact: true }),
  ).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(
    page.getByText("合成测试数据 · 不代表真实接入或结算"),
  ).toBeVisible();
  await expect(
    page.getByText("待重启 / 载入未确认", { exact: true }),
  ).toHaveCount(2);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({
    path: ".cache/evidence/access-light.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "接入 Orbit" }).first().click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("/fixture/config")).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "确认保存本地配置" }),
  ).toBeEnabled();
  const changed = bootstrap("synthetic-account-a", 2);
  changed.connections[0]!.revision = 2;
  await pushState(page, changed);
  await expect(
    dialog.getByRole("button", { name: "确认保存本地配置" }),
  ).toBeDisabled();
  expect(await calls(page, "apply_client_config")).toBe(0);
});

test("changing accounts clears prompt and drafts before another submit", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/images");
  await choose(page, "image-model", "synthetic-image");
  await page
    .locator("#image-prompt")
    .fill("Private synthetic account A prompt");
  await pushState(page, bootstrap("synthetic-account-b", 2));
  await expect(page.locator("#image-prompt")).toHaveValue("");
  await expect(page.locator("#image-model")).toHaveText("选择获准的模型");
  expect(await calls(page, "create_image_task")).toBe(0);
});

test("healthy session rotation preserves a draft while disabling submission, and expiry clears it", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/images");
  await choose(page, "image-model", "synthetic-image");
  await page.locator("#image-prompt").fill("Same actor draft during rotation");
  const rotating = bootstrap("synthetic-account-a", 2);
  rotating.session.status = "refreshing";
  await pushState(page, rotating);
  await expect(page.locator("#image-prompt")).toHaveValue(
    "Same actor draft during rotation",
  );
  await expect(page.locator("#image-model")).toHaveText("synthetic-image");
  await expect(
    page.getByRole("button", { name: "生成图片", exact: true }),
  ).toBeDisabled();
  await pushState(page, bootstrap("synthetic-account-a", 3));
  await expect(page.locator("#image-prompt")).toHaveValue(
    "Same actor draft during rotation",
  );
  await expect(
    page.getByRole("button", { name: "生成图片", exact: true }),
  ).toBeEnabled();
  const expired = bootstrap("synthetic-account-a", 4);
  expired.session.status = "expired";
  await pushState(page, expired);
  await expect(page.locator("#image-prompt")).toHaveValue("");
  await expect(page.locator("#image-model")).toHaveText("选择获准的模型");
  expect(await calls(page, "create_image_task")).toBe(0);
});

test("browser authorization remains pending and supports explicit denial and cancellation", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/");
  await expect(page.locator(".account-summary h2")).toHaveText(
    "Synthetic synthetic-account-a",
  );
  await expect(
    page.getByText("USD 123,456,789.1235", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "连接账户", exact: true }),
  ).toHaveCount(0);
  const expired = bootstrap("synthetic-account-a", 2);
  expired.session.status = "expired";
  expired.session.account = null;
  await pushState(page, expired);
  await page
    .getByRole("button", { name: "连接账户", exact: true })
    .last()
    .click();
  const dialog = page.getByRole("dialog");
  await expect(
    dialog.getByText("https://orivelle.powerbyte.io", { exact: true }),
  ).toHaveCount(0);
  await expect(dialog.locator("input, textarea, select")).toHaveCount(0);
  await dialog.getByRole("button", { name: "继续", exact: true }).click();
  await expect(
    dialog.getByRole("button", { name: "继续", exact: true }),
  ).toBeDisabled();
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __ORBIT_TEST__: FixtureControl }).__ORBIT_TEST__
          .lastRequests.start_login,
    ),
  ).toEqual({});
  await page.evaluate(() =>
    (
      window as unknown as { __ORBIT_TEST__: FixtureControl }
    ).__ORBIT_TEST__.pushAuthorization({
      request_id: "synthetic-login",
      state: "denied",
      error: null,
    }),
  );
  await expect(
    page.getByText("浏览器授权已拒绝，当前账户保持原状态。"),
  ).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "继续", exact: true }),
  ).toBeEnabled();
  await dialog.getByRole("button", { name: "继续", exact: true }).click();
  await expect(
    dialog.getByRole("button", { name: "继续", exact: true }),
  ).toBeDisabled();
  await dialog.getByRole("button", { name: "取消", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await calls(page, "cancel_login")).toBe(1);
  await page.evaluate(() =>
    (
      window as unknown as { __ORBIT_TEST__: FixtureControl }
    ).__ORBIT_TEST__.pushAuthorization({
      request_id: "synthetic-login",
      state: "connected",
      error: null,
    }),
  );
  await expect(page.locator(".account-summary h2")).toHaveText(
    "连接账户后开始",
  );
  expect(await calls(page, "create_image_task")).toBe(0);
});

test("synthetic Windows capabilities never offer an inaccessible hidden background window", async ({
  page,
}) => {
  const state = bootstrap();
  state.platform = "windows";
  await fixture(page, { state });
  await page.goto("/#/settings");
  await page.locator("#setting-close").click();
  await expect(
    page.getByRole("option", { name: "保留进程继续当前任务" }),
  ).toHaveCount(0);
  await expect(page.getByRole("option")).toHaveCount(2);
});

test("update maintenance disables image submission and settings without losing a same-account draft", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/images");
  await choose(page, "image-model", "synthetic-image");
  await page
    .locator("#image-prompt")
    .fill("Draft retained through maintenance");
  const state = bootstrap("synthetic-account-a", 2);
  state.maintenance_active = true;
  await pushState(page, state);
  await expect(
    page.getByRole("button", { name: "生成图片", exact: true }),
  ).toBeDisabled();
  await expect(page.locator("#image-prompt")).toHaveValue(
    "Draft retained through maintenance",
  );
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "设置", exact: true })
    .click();
  await expect(page.locator("#setting-theme")).toBeDisabled();
  await expect(
    page.getByText("正在准备安装更新，新的图片请求与配置变更已暂停。"),
  ).toBeVisible();
  expect(await calls(page, "create_image_task")).toBe(0);
});

for (const client of ["codex", "claude"] as const) {
  for (const recoverable of [false, true]) {
    test(`${client === "claude" ? "Claude" : "Codex"} conflicted backup offers a gated preview while logged out; recoverable=${recoverable}`, async ({
      page,
    }) => {
      const state = bootstrap();
      state.session.status = "logged_out";
      state.session.account = null;
      state.session.inference_credential_status = "unavailable";
      if (client === "claude") state.clients[1]!.managed = "unknown";
      state.backups = [
        {
          id: "synthetic-conflict-backup",
          client,
          account_id: "synthetic-account-a",
          account_label: "Fixture A",
          panel_origin: "https://panel.invalid",
          installation_id: "fixture-install",
          created_at: "2026-10-13T03:00:00Z",
          expires_at: "2026-11-12T03:00:00Z",
          file_count: client === "claude" ? 3 : 2,
          state: "conflict",
        },
      ];
      await fixture(page, {
        state,
        restoreMode: recoverable ? "recoverable" : "blocked",
      });
      await page.goto("/#/access");
      await choose(page, "backup-account", "Fixture A");
      await page.getByRole("button", { name: "预览恢复", exact: true }).click();
      const apply = page
        .getByRole("dialog")
        .getByRole("button", { name: "确认恢复配置", exact: true });
      if (client === "claude") {
        await expect(
          page.getByRole("dialog").locator(".path-list li"),
        ).toHaveCount(3);
        await expect(
          page.getByRole("dialog").getByText("Fixture A", { exact: true }),
        ).toBeVisible();
      }
      if (recoverable) {
        await expect(apply).toBeEnabled();
        await apply.click();
        expect(await calls(page, "restore_client_config")).toBe(1);
        await expect(page.getByRole("dialog")).toHaveCount(0);
        await expect(
          page.getByText(
            "原配置已恢复。请完整退出目标应用并重新打开，载入和连接仍待确认。",
            { exact: true },
          ),
        ).toBeVisible();
      } else {
        await expect(apply).toBeDisabled();
        expect(await calls(page, "restore_client_config")).toBe(0);
      }
      expect(await calls(page, "create_image_task")).toBe(0);
      expect(await calls(page, "apply_client_config")).toBe(0);
    });
  }
}

test("Claude diagnostic export requires confirmation and never claims configuration was applied", async ({
  page,
}) => {
  const state = bootstrap();
  state.clients[1]!.configuration_state = "not_configured";
  state.clients[1]!.loaded_state = "unconfirmed";
  state.clients[1]!.configuration_access = "unverified";
  state.clients[1]!.configuration_access_revision = null;
  await fixture(page, { state, claudeImport: true });
  await page.goto("/#/access");
  await chooseClaudeRoles(page);
  await page.getByRole("button", { name: "接入 Orbit" }).last().click();
  const dialog = page.getByRole("dialog");
  await expect(
    dialog.getByRole("button", { name: "确认保存本地配置" }),
  ).toBeDisabled();
  await expect(dialog.getByText(/不代表可直接导入/)).toBeVisible();
  const exportButton = dialog.getByRole("button", {
    name: "确认导出 Claude 诊断候选",
  });
  await expect(exportButton).toBeEnabled();
  expect(await calls(page, "export_claude_import")).toBe(0);
  await exportButton.click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(
    page.getByText(
      "Claude 配置诊断候选已导出；客户端配置、载入和连接状态未改变。",
    ),
  ).toBeVisible();
  expect(await calls(page, "apply_client_config")).toBe(0);
  expect(await calls(page, "export_claude_import")).toBe(1);
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __ORBIT_TEST__: FixtureControl }).__ORBIT_TEST__
          .lastRequests.export_claude_import,
    ),
  ).toEqual({ preview_id: "synthetic-preview", confirmed: true });
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "首页", exact: true })
    .click();
  await expect(page.getByText("配置已保存", { exact: true })).toHaveCount(1);
  await expect(page.getByText("尚未配置", { exact: true })).toHaveCount(1);
});

test("Claude local configuration save shows the complete impact without claiming load or connection", async ({
  page,
}) => {
  const state = bootstrap();
  state.clients[1]!.managed = "unknown";
  state.clients[1]!.configuration_state = "not_configured";
  state.clients[1]!.loaded_state = "unconfirmed";
  await fixture(page, { state });
  await page.goto("/#/access");
  await chooseClaudeRoles(page);
  await page.getByRole("button", { name: "接入 Orbit" }).last().click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.locator(".path-list li")).toHaveCount(3);
  await expect(dialog.getByText(/包括 Chat、Code 与 Cowork/)).toBeVisible();
  await expect(dialog.getByText(/远端组织策略尚未确认/)).toBeVisible();
  expect(await calls(page, "apply_client_config")).toBe(0);
  await dialog.getByRole("button", { name: "确认保存本地配置" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await calls(page, "apply_client_config")).toBe(1);
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __ORBIT_TEST__: FixtureControl }).__ORBIT_TEST__
          .lastRequests.apply_client_config,
    ),
  ).toEqual({ preview_id: "synthetic-preview", confirmed: true });
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "首页", exact: true })
    .click();
  const row = page.locator(".client-row").filter({
    has: page.getByRole("heading", { name: "Claude Desktop", exact: true }),
  });
  await expect(row.getByText("配置已保存", { exact: true })).toBeVisible();
  await expect(
    row
      .locator(".client-stages")
      .getByText("待重启 / 载入未确认", { exact: true }),
  ).toBeVisible();
  await expect(row.getByText("连接未验证", { exact: true })).toBeVisible();
  await expect(row.getByText("客户端载入已确认", { exact: true })).toHaveCount(
    0,
  );
  await expect(row.getByText("连接验证通过", { exact: true })).toHaveCount(0);
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "接入", exact: true })
    .click();
  await expect(page.locator(".backup-row")).toHaveCount(0);
  await page.getByRole("button", { name: "切回原配置", exact: true }).click();
  await dialog.getByRole("button", { name: "确认恢复配置" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "切回原配置", exact: true }),
  ).toHaveCount(0);
  expect(await calls(page, "restore_client_config")).toBe(1);
});

test("Claude qualification changes permanently retire the old confirmation", async ({
  page,
}) => {
  const state = bootstrap();
  state.clients[1]!.managed = "unknown";
  await fixture(page, { state });
  await page.goto("/#/access");
  await chooseClaudeRoles(page);
  await page.getByRole("button", { name: "接入 Orbit" }).last().click();
  const button = page
    .getByRole("dialog")
    .getByRole("button", { name: "确认保存本地配置" });
  await expect(button).toBeEnabled();
  const changed = bootstrap("synthetic-account-a", 2);
  changed.clients[1]!.configuration_access_revision =
    "synthetic-new-claude-target";
  await pushState(page, changed);
  await expect(button).toBeDisabled();
  await pushState(page, bootstrap("synthetic-account-a", 3));
  await expect(button).toBeDisabled();
  expect(await calls(page, "apply_client_config")).toBe(0);
});

test("UTF-8 bound prevents a paid submit and failed paid submission never retries", async ({
  page,
}) => {
  await fixture(page, { paidFailure: true });
  await page.goto("/#/images");
  await choose(page, "image-model", "synthetic-image");
  await page.locator("#image-prompt").fill("中".repeat(10923));
  await expect(
    page.getByRole("button", { name: "生成图片", exact: true }),
  ).toBeDisabled();
  expect(await calls(page, "create_image_task")).toBe(0);
  await expect(page.locator("#image-quality")).toHaveCount(0);
  await page.getByRole("button", { name: "高级选项" }).click();
  await choose(page, "image-background", "透明");
  await page.locator("#image-prompt").fill("Synthetic image prompt");
  await page.getByRole("button", { name: "生成图片", exact: true }).click();
  await expect(
    page.getByText("结果待确认", { exact: true }).first(),
  ).toBeVisible();
  await expect(page.getByText("费用待确认", { exact: true })).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __ORBIT_TEST__: FixtureControl }).__ORBIT_TEST__
          .lastRequests.create_image_task?.parameters,
    ),
  ).toEqual({ background: "transparent" });

  await expect(
    page.getByText("SYNTHETIC_SECRET_SHOULD_NOT_RENDER"),
  ).toHaveCount(0);
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "首页", exact: true })
    .click();
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "图片工作台", exact: true })
    .click();
  expect(await calls(page, "create_image_task")).toBe(1);
  await expect(page.getByRole("button", { name: "重试生成" })).toHaveCount(0);
});

test("image background defaults to opaque and is submitted while advanced options stay collapsed", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/images");
  await choose(page, "image-model", "synthetic-image");
  const advanced = page.getByRole("button", { name: "高级选项" });
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(".parameter-details")).toContainText("不透明");
  await advanced.click();
  await expect(page.locator("#image-background")).toContainText("不透明");
  await page.locator("#image-background").click();
  await expect(page.getByRole("option")).toHaveText(["不透明", "透明"]);
  await page.keyboard.press("Escape");
  await advanced.click();
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await page
    .locator("#image-prompt")
    .fill("Synthetic opaque background request");
  await page.getByRole("button", { name: "生成图片", exact: true }).click();
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __ORBIT_TEST__: FixtureControl }).__ORBIT_TEST__
          .lastRequests.create_image_task?.parameters,
    ),
  ).toEqual({ background: "opaque" });
  expect(await calls(page, "create_image_task")).toBe(1);
});

test("available image controls reject legacy catalog choices and sanitize history reuse", async ({
  page,
}) => {
  const state = bootstrap();
  const historical = task("completed");
  historical.prompt = "Synthetic historical request";
  historical.parameters = {
    n: 3,
    quality: "max",
    size: "1536x864",
    output_format: "webp",
    output_compression: 0,
    complexity: "high",
    background: "transparent",
  };
  state.tasks = [historical];
  await fixture(page, { state });
  await page.goto("/#/images");
  await choose(page, "image-model", "synthetic-image");
  const advanced = page.getByRole("button", { name: "高级选项" });
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await advanced.click();
  await page.getByRole("button", { name: "复用参数", exact: true }).click();
  await expect(page.locator("#image-prompt")).toHaveValue(historical.prompt);
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(".parameter-details")).toContainText("透明");
  await advanced.click();
  await expect(page.locator("#image-background")).toContainText("透明");
  for (const id of [
    "image-n",
    "image-size",
    "image-width",
    "image-height",
    "image-quality",
    "image-output_format",
    "image-compression",
    "image-complexity",
  ])
    await expect(page.locator("#" + id)).toHaveCount(0);
  await expect(page.locator(".parameter-details")).toContainText(
    "透明背景可能增加图片 token 用量",
  );
  // Historical facts remain readable even though their controls cannot be submitted again.
  await expect(page.locator(".task-facts")).toContainText("1536x864");
  await page.locator(".parameter-details").screenshot({
    path: ".cache/evidence/images-background-explicit-choice.png",
  });
  await advanced.click();
  await expect(page.locator("#image-background")).toHaveCount(0);
  await page.getByRole("button", { name: "生成图片", exact: true }).click();
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __ORBIT_TEST__: FixtureControl }).__ORBIT_TEST__
          .lastRequests.create_image_task?.parameters,
    ),
  ).toEqual({ background: "transparent" });
  expect(await calls(page, "create_image_task")).toBe(1);
});

test("crash-recovered local materials expose only a local saving retry", async ({
  page,
}) => {
  const state = bootstrap();
  const recovered = task("unconfirmed");
  recovered.local_save_available = true;
  state.tasks = [recovered];
  await fixture(page, { state });
  await page.goto("/#/images");
  await page.getByRole("button", { name: "重试本地保存" }).click();
  await expect(page.locator(".task-header .badge")).toHaveText(
    "本地结果已保存",
  );
  expect(await calls(page, "retry_local_save")).toBe(1);
  expect(await calls(page, "create_image_task")).toBe(0);
  await expect(page.getByText("费用待确认", { exact: true })).toBeVisible();
});

test("deleting one local image never deletes other results of the same task", async ({
  page,
}) => {
  const state = bootstrap();
  state.tasks = [task()];
  state.tasks[0]!.image_ids = ["image-1", "image-2"];
  state.tasks[0]!.parameters = {
    size: "1536x864",
    quality: "max",
    output_format: "webp",
  };
  state.tasks[0]!.result_parameters = {
    size: "1254x1254",
    quality: "high",
    output_format: "png",
    background: null,
    observed_model: "gpt-image-2.5-flare",
  };
  state.image_history = ["image-1", "image-2"].map((id) => ({
    id,
    task_id: "synthetic-task",
    account_id: "synthetic-account-a",
    created_at: "2026-10-13",
    requested_model: "synthetic-image",
    observed_model: null,
    mime: "image/png",
    width: 16,
    height: 16,
    bytes: 100,
    preview_url: null,
  }));
  await fixture(page, { state });
  await page.goto("/#/images");
  await expect(page.locator(".task-facts")).toContainText("1536x864");
  await expect(page.locator(".task-facts")).toContainText("1254x1254");
  await page.getByRole("tab", { name: "本地历史" }).click();
  await expect(page.locator(".history-card")).toHaveCount(2);
  await expect(page.locator(".history-card").first()).toContainText(
    "16 × 16 · PNG",
  );
  await expect(page.locator(".history-card").first()).toContainText("1536x864");
  await expect(page.locator(".history-card").first()).toContainText(
    "1254x1254",
  );
  await page
    .getByRole("button", { name: "删除本地历史", exact: true })
    .first()
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "确认", exact: true })
    .click();
  await expect(page.locator(".history-card")).toHaveCount(1);
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __ORBIT_TEST__: FixtureControl }).__ORBIT_TEST__
          .lastRequests.delete_history,
    ),
  ).toEqual({ image_ids: ["image-1"], confirmed: true });
  expect(await calls(page, "create_image_task")).toBe(0);
});

test("access and settings show the supported controls and the installed version", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/access");
  await expect(
    page.getByRole("heading", { name: "角色模型", exact: true }),
  ).toBeVisible();
  await expect(page.getByText("更多操作", { exact: true })).toHaveCount(0);
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "设置", exact: true })
    .click();
  const currentVersion = page.locator(".settings-current-version");
  await expect(
    currentVersion.getByText("当前版本", { exact: true }),
  ).toBeVisible();
  await expect(currentVersion.locator("strong")).toHaveText(manifest.version);
  expect(await calls(page, "check_update")).toBe(0);
  await expect(
    page.getByRole("button", { name: "退出登录", exact: true }),
  ).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "忘记此安装", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByText("Orivelle Orbit · 2 GiB", { exact: true }),
  ).toHaveCount(0);
});

test("theme, language, wide window and accessible image tabs", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/settings");
  await choose(page, "setting-theme", "深色");
  await expect(page.locator("html")).toHaveClass("dark");
  await choose(page, "setting-locale", "English");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(
    "Make Orbit fit your workflow",
  );
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "Images", exact: true })
    .click();
  const create = page.getByRole("tab", { name: "Create", exact: true });
  await page.screenshot({
    path: ".cache/evidence/images-composer-dark-wide.png",
    fullPage: true,
  });
  await create.focus();
  await page.keyboard.press("ArrowRight");
  await expect(page.getByRole("tab", { name: "Local history" })).toBeFocused();
  await page.screenshot({
    path: ".cache/evidence/images-dark-wide.png",
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});
