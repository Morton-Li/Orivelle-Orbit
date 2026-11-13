import { describe, expect, it } from "vitest";
import type {
  ImageParameters,
  ImageParameterConstraints,
} from "../../src/shared/contracts";
import {
  imageParameterError,
  supportedImageParameters,
  imageFailureMessage,
  availableImageConstraints,
  imageParameterFields,
} from "../../src/shared/image-parameters";
import { publicFailure } from "../../src/shared/bridge";
import fixture from "../contracts/images-parameters.json" with { type: "json" };

const caps = fixture.models["gpt-image-2.5-flare"] as ImageParameterConstraints;
describe("declared static image parameters", () => {
  it("restricts a legacy catalog and reused parameters to available product controls", () => {
    const available = availableImageConstraints(caps);
    expect(available).toEqual(fixture.codex_available_controls);
    expect(imageParameterFields(available).map((field) => field.key)).toEqual([
      "background",
    ]);
    const oldParameters: ImageParameters = {
      n: 3,
      quality: "max",
      size: "1536x864",
      output_format: "webp",
      output_compression: 0,
      complexity: "high",
      background: "transparent",
    };
    expect(supportedImageParameters(oldParameters, available)).toEqual({
      background: "transparent",
    });
    expect(imageParameterError(oldParameters, available)).not.toBeNull();
    expect(
      imageParameterError({ background: "transparent" }, available),
    ).toBeNull();
    expect(caps.output_formats).toContain("webp");
    expect(caps.max_results).toBe(5);
  });
  it("accepts shared valid requests and rejects invalid combinations before a paid submit", () => {
    for (const request of fixture.valid_requests) {
      const parameters = Object.fromEntries(
        Object.entries(request).filter(
          ([key]) => key !== "model" && key !== "prompt",
        ),
      );
      expect(
        imageParameterError(
          parameters as ImageParameters,
          fixture.models[
            request.model as keyof typeof fixture.models
          ] as ImageParameterConstraints,
        ),
      ).toBeNull();
    }
    for (const parameters of fixture.invalid_requests) {
      if ("mask" in parameters || "stream" in parameters) continue; // Native IPC rejects unknown request fields.
      expect(
        imageParameterError(parameters as ImageParameters, caps),
        JSON.stringify(parameters),
      ).not.toBeNull();
    }
  });
  it("clears unsupported reused values and keeps explicit zero compression", () => {
    expect(
      supportedImageParameters(
        {
          size: "1536x864",
          output_format: "webp",
          output_compression: 0,
          quality: "max",
        },
        caps,
      ),
    ).toEqual({
      size: "1536x864",
      output_format: "webp",
      output_compression: 0,
      quality: "max",
    });
    expect(
      supportedImageParameters(
        { quality: "max", complexity: "high" },
        fixture.models["gpt-image-2"] as ImageParameterConstraints,
      ),
    ).toEqual({});
    const oldCaps = {
      ...caps,
      contract: undefined,
      output_formats: [],
      complexities: [],
      output_compression: false,
      custom_size: null,
    };
    expect(
      imageParameterError({ quality: "high", size: "1024x1024" }, oldCaps),
    ).toBeNull();
    expect(
      imageParameterError({ output_format: "png" }, oldCaps),
    ).not.toBeNull();
  });
  it("projects safe local messages without exposing provider content", () => {
    const failure = publicFailure({
      code: "image_access_denied",
      message: "SYNTHETIC_PRIVATE_PROVIDER_MESSAGE",
      retryable: false,
    });
    expect(failure.message).toBe(imageFailureMessage("image_access_denied"));
    expect(failure.message).not.toContain("SYNTHETIC_PRIVATE");
    expect(imageFailureMessage("unrecognized_private_error")).not.toContain(
      "unrecognized_private_error",
    );
  });
});
