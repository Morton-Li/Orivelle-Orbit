import { describe, expect, it } from "vitest";
import type {
  ImageApiParameters,
  ImageParameters,
  ImageParameterConstraints,
} from "../../src/shared/contracts";
import {
  workbenchConstraints,
  workbenchParameterError,
  adjustedImageParameters,
} from "../../src/shared/image-api";
import { publicFailure } from "../../src/shared/bridge";
import catalog from "../contracts/images-api-catalog.json" with { type: "json" };
import legacy from "../contracts/images-parameters.json" with { type: "json" };
const api = catalog.models["gpt-image-2.6-flare"] as ImageApiParameters;
const subscription =
  legacy.codex_available_controls as ImageParameterConstraints;
describe("image API workbench", () => {
  it("only explicit API metadata exposes more controls and preserves the subscription catalog", () => {
    const plain = workbenchConstraints(subscription, null)!;
    expect(plain.max_results).toBe(1);
    expect(plain.output_formats).toEqual([]);
    const full = workbenchConstraints(subscription, api)!;
    expect(full.max_results).toBe(10);
    expect(full.max_inputs).toBe(16);
    expect(full.output_formats).toContain("svg");
    expect(subscription.max_results).toBe(1);
    expect(
      workbenchConstraints(subscription, {
        ...api,
        contract: "future",
      } as unknown as ImageApiParameters),
    ).toEqual(plain);
  });
  it("defaults stay on subscription while only actual changed controls are reported", () => {
    const defaults: ImageParameters = {
      n: 1,
      quality: "auto",
      size: "auto",
      output_format: "png",
      background: "transparent",
      stream: false,
      partial_images: 0,
      moderation: "auto",
    };
    expect(adjustedImageParameters(defaults, 5, "ordinary")).toEqual([]);
    expect(
      workbenchParameterError(
        defaults,
        workbenchConstraints(subscription, api),
        api,
        5,
      ),
    ).toBeNull();
    expect(
      adjustedImageParameters(
        { ...defaults, quality: "high", size: "1536x864" },
        1,
        "ordinary",
      ),
    ).toEqual(["quality", "size"]);
    expect(
      adjustedImageParameters({
        output_format: "gif",
        frame_count: 9,
        frame_duration_ms: 200,
      }),
    ).toEqual(["output_format"]);
    expect(adjustedImageParameters({}, 16, "ordinary", true)).toEqual([
      "mask",
      "images",
    ]);
  });
  it("validates formats, edits and streaming together before submit", () => {
    const caps = workbenchConstraints(subscription, api);
    for (const values of [
      { output_format: "svg", stream: true },
      { output_format: "svg", n: 2 },
      { partial_images: 1 },
      { output_format: "gif", output_compression: 100 },
      { frame_count: 9 },
      { output_format: "gif", frame_count: 17 },
    ] as ImageParameters[]) {
      expect(
        workbenchParameterError(values, caps, api, 0),
        JSON.stringify(values),
      ).not.toBeNull();
    }
    expect(
      workbenchParameterError({ output_format: "svg" }, caps, api, 1),
    ).not.toBeNull();
    expect(
      workbenchParameterError({ output_format: "svg" }, caps, api, 0),
    ).toBeNull();
    expect(
      workbenchParameterError(
        { output_format: "gif", size: "512x512", frame_count: 16 },
        caps,
        api,
        0,
      ),
    ).toBeNull();
    expect(
      workbenchParameterError(
        { stream: true, partial_images: 3 },
        caps,
        api,
        0,
      ),
    ).toBeNull();
    expect(
      workbenchParameterError({}, caps, api, 1, true, true),
    ).not.toBeNull();
    expect(
      workbenchParameterError({ input_fidelity: "high" }, caps, api, 1),
    ).not.toBeNull();
  });
  it("keeps the native safe field list and discards arbitrary error text and keys", () => {
    const error = publicFailure({
      code: "image_parameters_unavailable",
      message: "PRIVATE_PROVIDER_BODY",
      unsupported_parameters: ["quality", "size", "PRIVATE_KEY"],
      retryable: false,
    });
    expect(error.message).toContain("质量");
    expect(error.message).toContain("尺寸");
    expect(error.message).not.toContain("PRIVATE");
    expect(error.message).not.toContain("背景");
    expect(error.unsupportedParameters).toEqual(["quality", "size"]);
  });
});
