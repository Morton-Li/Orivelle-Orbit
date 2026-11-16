import type {
  ImageApiParameters,
  ImageParameters,
  ImageParameterConstraints,
} from "./contracts";
import type { MessageKey } from "./i18n";
import {
  availableImageConstraints,
  IMAGE_API_CONTRACT,
  imageParameterError,
} from "./image-parameters";
export function workbenchConstraints(
  subscription: ImageParameterConstraints | null,
  api: ImageApiParameters | null | undefined,
): ImageParameterConstraints | null {
  if (
    api?.contract !== IMAGE_API_CONTRACT ||
    api.parameters.contract !== IMAGE_API_CONTRACT
  )
    return availableImageConstraints(subscription);
  return {
    ...api.parameters,
    max_results: Math.min(10, api.parameters.max_results),
    max_inputs: Math.min(16, api.parameters.max_inputs),
    backgrounds: api.parameters.backgrounds.filter((v) =>
      ["opaque", "transparent"].includes(v),
    ),
  };
}
export function adjustedImageParameters(
  p: ImageParameters,
  inputs = 0,
  prompt = "",
  mask = false,
  markup = false,
): string[] {
  const controls: [string, boolean][] = [
    ["n", (p.n ?? 1) > 1],
    ["quality", p.quality !== undefined && p.quality !== "auto"],
    ["size", p.size !== undefined && p.size !== "auto"],
    [
      "output_format",
      p.output_format !== undefined && p.output_format !== "png",
    ],
    [
      "output_compression",
      p.output_compression !== undefined && p.output_compression !== 100,
    ],
    ["complexity", p.complexity !== undefined],
    ["mask", mask],
    ["markup", markup],
    ["stream", p.stream === true],
    ["partial_images", (p.partial_images ?? 0) > 0],
    ["moderation", p.moderation !== undefined && p.moderation !== "auto"],
    [
      "input_fidelity",
      p.input_fidelity !== undefined && p.input_fidelity !== "low",
    ],
    ["frame_count", p.frame_count !== undefined && p.frame_count !== 9],
    [
      "frame_duration_ms",
      p.frame_duration_ms !== undefined && p.frame_duration_ms !== 200,
    ],
    ["svg", p.svg !== undefined],
    ["images", inputs > 5],
    ["prompt", new TextEncoder().encode(prompt).length > 32768],
  ];
  return controls.filter(([, changed]) => changed).map(([key]) => key);
}
export function workbenchParameterError(
  p: ImageParameters,
  caps: ImageParameterConstraints | null,
  api: ImageApiParameters | null | undefined,
  inputs: number,
  mask = false,
  markup = false,
): MessageKey | null {
  const format = p.output_format ?? "png";
  const stream = p.stream ?? false;
  if (
    (mask && markup) ||
    ((mask || markup || p.input_fidelity !== undefined) && !inputs)
  )
    return "images.invalidMaterial";
  if (
    (format === "svg" && (inputs > 0 || (p.n ?? 1) !== 1)) ||
    (p.svg !== undefined && format !== "svg")
  )
    return "images.invalidSvg";
  if (
    (["svg", "gif"].includes(format) &&
      (stream ||
        (p.partial_images ?? 0) > 0 ||
        p.output_compression !== undefined)) ||
    (format === "gif" && mask)
  )
    return "images.invalidMediaOptions";
  if (
    ((p.partial_images ?? 0) > 0 && !stream) ||
    (p.partial_images ?? 0) < 0 ||
    (p.partial_images ?? 0) > 3 ||
    !Number.isInteger(p.partial_images ?? 0)
  )
    return "images.invalidParameters";
  if (
    ((p.frame_count !== undefined || p.frame_duration_ms !== undefined) &&
      format !== "gif") ||
    (p.frame_count !== undefined &&
      (!Number.isInteger(p.frame_count) ||
        p.frame_count < 2 ||
        p.frame_count > 16)) ||
    (p.frame_duration_ms !== undefined &&
      (!Number.isInteger(p.frame_duration_ms) ||
        p.frame_duration_ms < 20 ||
        p.frame_duration_ms > 655350))
  )
    return "images.invalidMediaOptions";
  if (
    p.svg &&
    (!/^[A-Za-z0-9][A-Za-z0-9._:/@-]{0,254}$/.test(p.svg.model) ||
      (p.svg.reasoning?.effort !== undefined &&
        !api?.svg_reasoning_efforts.includes(p.svg.reasoning.effort)))
  )
    return "images.svgAuthorRequired";
  if (
    (mask && !api?.mask) ||
    (markup && !api?.markup) ||
    (stream && !api?.stream) ||
    (p.input_fidelity !== undefined &&
      p.input_fidelity !== "low" &&
      !api?.input_fidelities.includes(p.input_fidelity)) ||
    (p.moderation !== undefined &&
      p.moderation !== "auto" &&
      !api?.moderations.includes(p.moderation))
  )
    return "images.invalidParameters";
  let common = p;
  if (format === "gif" && p.size && p.size !== "auto") {
    const match = /^(\d{2,4})x(\d{2,4})$/.exec(p.size);
    const width = Number(match?.[1]),
      height = Number(match?.[2]);
    if (
      !match ||
      width < 16 ||
      height < 16 ||
      width > 3840 ||
      height > 3840 ||
      width % 16 ||
      height % 16
    )
      return "images.invalidSize";
    common = { ...p, size: "auto" };
  }
  if (p.background === "transparent" && !["png", "webp"].includes(format))
    return "images.transparentFormat";
  return imageParameterError(common, caps);
}
