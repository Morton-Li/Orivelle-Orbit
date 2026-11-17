import { expect, test, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import type {
  ImageApiParameters,
  ImageInput,
  HistoryImage,
} from "../../src/shared/contracts";
import catalog from "../contracts/images-api-catalog.json" with { type: "json" };
import { bootstrap, imageModel, task } from "../fixtures";
import {
  calls,
  fixture,
  pushState,
  type FixtureControl,
} from "./native-fixture";

const model = {
  ...imageModel(),
  image_api_parameters: catalog.models[
    "gpt-image-2.6-flare"
  ] as ImageApiParameters,
};
async function choose(page: Page, id: string, name: string) {
  await page.locator(`#${id}`).click();
  await page.getByRole("option", { name, exact: true }).click();
}
async function request(page: Page) {
  return page.evaluate(
    () =>
      (window as unknown as { __ORBIT_TEST__: FixtureControl }).__ORBIT_TEST__
        .lastRequests.create_image_task,
  );
}
function input(id: string): ImageInput {
  return {
    id,
    name: `${id}.png`,
    mime: "image/png",
    width: 16,
    height: 16,
    bytes: 100,
    preview_url: null,
  };
}

test("API catalog keeps only background open without changing subscription defaults", async ({
  page,
}) => {
  await fixture(page, { models: [model] });
  await page.goto("/#/images");
  await choose(page, "image-model", "synthetic-image");
  await expect(page.locator(".image-route-hint")).toHaveText(
    "本次使用订阅图片服务",
  );
  await page.getByRole("button", { name: "高级选项" }).click();
  await expect(page.locator("#image-parameters .field-label")).toHaveText([
    "背景",
  ]);
  await expect(page.locator("#image-parameters input")).toHaveCount(0);
  await expect(page.locator("#image-background")).toContainText("不透明");
  await choose(page, "image-background", "透明");
  await expect(page.locator(".image-route-hint")).toHaveText(
    "本次使用订阅图片服务",
  );
  await page.locator(".parameter-details").screenshot({
    path: ".cache/evidence/images-api-options.png",
  });
  await page.getByRole("button", { name: "高级选项" }).click();
  await page.locator("#image-prompt").fill("Synthetic explicit defaults");
  await page.getByRole("button", { name: "生成图片", exact: true }).click();
  expect((await request(page))?.parameters).toEqual({
    background: "transparent",
  });
  expect(await calls(page, "create_image_task")).toBe(1);
});

test("Unicode prompt counts keep the subscription byte boundary and API character limit distinct", async ({
  page,
}) => {
  await fixture(page, { models: [model] });
  await page.goto("/#/images");
  await choose(page, "image-model", "synthetic-image");
  const prompt = page.locator("#image-prompt");
  await prompt.fill("中".repeat(10000));
  await expect(page.locator(".image-route-hint")).toHaveText(
    "本次使用订阅图片服务",
  );
  await prompt.fill("中".repeat(10923));
  await expect(page.locator(".image-route-hint")).toContainText("高级图像 API");
  await expect(
    page.getByRole("button", { name: "生成图片", exact: true }),
  ).toBeEnabled();
  await prompt.fill("中".repeat(32001));
  await expect(
    page.getByRole("button", { name: "生成图片", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByText("提示词超过当前模型的字符上限。", { exact: true }),
  ).toBeVisible();
  expect(await calls(page, "create_image_task")).toBe(0);
});

test("editing keeps background available while mask and markup remain closed", async ({
  page,
}) => {
  await fixture(page, {
    models: [model],
    imageInputs: [input("reference")],
    imageMaterials: { mask: input("mask"), markup: input("markup") },
  });
  await page.goto("/#/images");
  await choose(page, "image-model", "synthetic-image");
  await page.getByRole("button", { name: "添加参考图", exact: true }).click();
  await page.getByRole("button", { name: "高级选项" }).click();
  await expect(page.locator("#image-parameters .field-label")).toHaveText([
    "背景",
  ]);
  await expect(page.locator(".image-materials")).toHaveCount(0);
  await choose(page, "image-background", "透明");
  await page.locator(".input-images .icon-button").click();
  await expect(page.locator(".image-route-hint")).toHaveText(
    "本次使用订阅图片服务",
  );
  await page.getByRole("button", { name: "添加参考图", exact: true }).click();
  await page.locator("#image-prompt").fill("Synthetic edit");
  await page.getByRole("button", { name: "编辑图片", exact: true }).click();
  expect(await request(page)).toMatchObject({
    input_ids: ["reference"],
    parameters: { background: "transparent" },
  });
  expect(await request(page)).not.toHaveProperty("mask_input_id");
  expect(await request(page)).not.toHaveProperty("markup_input_id");
  expect(await calls(page, "select_image_inputs")).toBe(2);
});

test("partial stream previews never become saved results and missing-pool errors name only adjusted parameters", async ({
  page,
}) => {
  const state = bootstrap();
  const active = {
    ...task("waiting"),
    parameters: { stream: true, partial_images: 1 },
    partial_preview: {
      id: "partial",
      index: 0,
      preview_url: "http://asset.localhost/partial",
    },
  };
  state.tasks = [active];
  await fixture(page, { state, models: [model] });
  await page.route("http://asset.localhost/**", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#cab8a4"/></svg>',
    }),
  );
  await page.goto("/#/images");
  await expect(page.locator(".partial-image-preview img")).toBeVisible();
  await expect(page.locator(".result-card")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "保存", exact: true }),
  ).toHaveCount(0);
  state.tasks = [
    {
      ...active,
      state: "failed",
      partial_preview: null,
      error: {
        code: "image_parameters_unavailable",
        message: "PRIVATE_PROVIDER_BODY",
        retryable: false,
        unsupported_parameters: ["stream", "partial_images"],
      },
    },
  ];
  state.state_revision++;
  await pushState(page, state);
  await expect(page.locator(".partial-image-preview")).toHaveCount(0);
  const error = page.locator(".image-result .error-text");
  await expect(error).toContainText("流式");
  await expect(error).toContainText("中间");
  await expect(error).not.toContainText(/质量|尺寸|PRIVATE/);
  await page.getByRole("tab", { name: "本地历史" }).click();
  await expect(page.locator(".history-card")).toHaveCount(0);
  expect(await calls(page, "create_image_task")).toBe(0);
});

for (const format of ["svg", "gif"] as const) {
  test(`${format} results keep preview, copy and original save actions with a disabled reference action`, async ({
    page,
  }) => {
    const animated = format === "gif";
    await page.setViewportSize({ width: animated ? 460 : 1280, height: 900 });
    const state = bootstrap();
    state.settings.theme = animated ? "dark" : "light";
    const pending = task("waiting");
    state.tasks = [pending];
    await fixture(page, { state, models: [model] });
    const preview = await readFile(
      new URL(
        `../contracts/images-api-preview.${animated ? "gif" : "png"}`,
        import.meta.url,
      ),
    );
    await page.route("http://asset.localhost/**", (route) =>
      route.fulfill({
        contentType: animated ? "image/gif" : "image/png",
        body: preview,
      }),
    );
    await page.goto("/#/images");
    const image: HistoryImage = {
      id: "api-media",
      task_id: pending.id,
      account_id: pending.account_id,
      requested_model: pending.requested_model,
      observed_model: null,
      created_at: "2026-11-16T03:00:00Z",
      mime: animated ? "image/gif" : "image/svg+xml",
      width: 16,
      height: 16,
      bytes: 467,
      preview_url: `http://asset.localhost/${animated ? "animated.gif" : "vector.png"}`,
    };
    state.tasks = [{ ...pending, state: "completed", image_ids: [image.id] }];
    state.image_history = [image];
    state.state_revision++;
    await pushState(page, state);
    const result = page.locator(".result-card");
    await expect(result.locator("img")).toHaveJSProperty("naturalWidth", 16);
    await expect(result.locator(".image-card-meta")).toContainText(
      format.toUpperCase(),
    );
    await expect(result).not.toContainText("SVG+XML");
    await expect(
      result.getByRole("button", { name: "再次编辑", exact: true }),
    ).toBeDisabled();
    const copy = animated ? "复制首帧" : "复制图片";
    await result.getByRole("button", { name: copy, exact: true }).click();
    await result.getByRole("button", { name: "保存", exact: true }).click();
    expect(await calls(page, "copy_image")).toBe(1);
    expect(await calls(page, "save_image_as")).toBe(1);
    await result.locator(".image-preview-button").click();
    await expect(page.locator(".zoom-image")).toHaveJSProperty(
      "naturalWidth",
      16,
    );
    if (animated)
      await expect(
        page.getByText("动图预览经过缩小；另存保留原尺寸及完整动画。", {
          exact: true,
        }),
      ).toBeVisible();
    await page
      .getByRole("dialog")
      .screenshot({ path: `.cache/evidence/images-api-${format}-preview.png` });
    await page.keyboard.press("Escape");
    await page.getByRole("tab", { name: "本地历史" }).click();
    await expect(page.locator(".history-card")).toContainText(
      format.toUpperCase(),
    );
    await expect(
      page
        .locator(".history-card")
        .getByRole("button", { name: "再次编辑", exact: true }),
    ).toBeDisabled();
    expect(await calls(page, "use_history_as_input")).toBe(0);
    expect(await calls(page, "create_image_task")).toBe(0);
  });
}
