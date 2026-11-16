import { expect, test, type Page } from "@playwright/test";
import type {
  ImageApiParameters,
  ImageInput,
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

test("API catalog exposes advanced controls without changing explicit subscription defaults", async ({
  page,
}) => {
  await fixture(page, { models: [model] });
  await page.goto("/#/images");
  await choose(page, "image-model", "synthetic-image");
  await expect(page.locator(".image-route-hint")).toHaveText(
    "本次使用订阅图片服务",
  );
  await page.getByRole("button", { name: "高级选项" }).click();
  await choose(page, "image-n", "1");
  await choose(page, "image-size", "自动");
  await choose(page, "image-quality", "自动");
  await choose(page, "image-output_format", "PNG");
  await choose(page, "image-background", "透明");
  await page.locator("#image-stream").check();
  await expect(page.locator(".image-route-hint")).toContainText("高级图像 API");
  await page.locator("#image-stream").uncheck();
  await expect(page.locator(".image-route-hint")).toHaveText(
    "本次使用订阅图片服务",
  );
  await page.getByRole("button", { name: "高级选项" }).click();
  await page.locator("#image-prompt").fill("Synthetic explicit defaults");
  await page.getByRole("button", { name: "生成图片", exact: true }).click();
  expect((await request(page))?.parameters).toEqual({
    background: "transparent",
    n: 1,
    size: "auto",
    quality: "auto",
    output_format: "png",
    stream: false,
    partial_images: 0,
  });
  expect(await calls(page, "create_image_task")).toBe(1);
});

test("editing selects scoped PNG mask or markup and clears it when the reference changes", async ({
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
  await page.getByRole("button", { name: "选择蒙版 PNG" }).click();
  await expect(page.locator(".image-materials")).toContainText("mask.png");
  await expect(page.locator(".image-route-hint")).toContainText("高级图像 API");
  await page.getByRole("button", { name: "选择标注 PNG" }).click();
  await expect(page.locator(".image-materials")).toContainText("markup.png");
  await expect(page.locator(".image-materials")).not.toContainText("mask.png");
  await page.locator(".input-images .icon-button").click();
  await expect(page.locator(".image-materials")).toHaveCount(0);
  await expect(page.locator(".image-route-hint")).toHaveText(
    "本次使用订阅图片服务",
  );
  await page.getByRole("button", { name: "添加参考图", exact: true }).click();
  await page.getByRole("button", { name: "选择标注 PNG" }).click();
  await page.locator("#image-prompt").fill("Synthetic edit");
  await page.getByRole("button", { name: "编辑图片", exact: true }).click();
  expect(await request(page)).toMatchObject({
    input_ids: ["reference"],
    markup_input_id: "markup",
  });
  expect(await request(page)).not.toHaveProperty("mask_input_id");
});

test("format controls enforce SVG and GIF combinations before a paid command", async ({
  page,
}) => {
  await fixture(page, { models: [model] });
  await page.goto("/#/images");
  await choose(page, "image-model", "synthetic-image");
  await page.getByRole("button", { name: "高级选项" }).click();
  await choose(page, "image-output_format", "GIF");
  await expect(page.locator("#image-stream")).toHaveCount(0);
  await expect(page.locator("#image-frame-count")).toHaveValue("9");
  await page.locator("#image-frame-count").fill("16");
  await page.locator("#image-frame-duration").fill("200");
  await choose(page, "image-output_format", "SVG");
  await expect(page.locator("#image-frame-count")).toHaveCount(0);
  await choose(page, "image-n", "2");
  await page.locator("#image-prompt").fill("Synthetic vector");
  await expect(
    page.getByRole("button", { name: "生成图片", exact: true }),
  ).toBeDisabled();
  await choose(page, "image-n", "1");
  await page.locator("#image-svg-model").fill("synthetic-vector-author");
  await expect(
    page.getByRole("button", { name: "生成图片", exact: true }),
  ).toBeEnabled();
  await page
    .locator(".parameter-details")
    .screenshot({ path: ".cache/evidence/images-api-options.png" });
  expect(await calls(page, "create_image_task")).toBe(0);
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
