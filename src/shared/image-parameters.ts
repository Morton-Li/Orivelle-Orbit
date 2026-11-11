import type {
  ImageParameterConstraints,
  ImageParameters,
  ImageResultParameters,
  ImageSizeConstraints,
} from "./contracts";
import { t, type MessageKey } from "./i18n";

export const IMAGE_PARAMETER_CONTRACT = "codex-images-static-parameters-v1";
export type ImageSelectParameter = Exclude<
  keyof ImageParameters,
  "output_compression"
>;

// A previous Panel or cached native catalog may still advertise the wider
// legacy wire subset. Product controls must not infer upstream behavior from it.
export function availableImageConstraints(
  caps: ImageParameterConstraints | null,
): ImageParameterConstraints | null {
  if (!caps) return null;
  return {
    ...caps,
    max_results: Math.min(1, caps.max_results),
    max_inputs: Math.min(5, caps.max_inputs),
    qualities: [],
    sizes: [],
    backgrounds: caps.backgrounds.filter((value) =>
      ["opaque", "transparent"].includes(value),
    ),
    output_formats: [],
    output_compression: false,
    complexities: [],
    custom_size: null,
  };
}

export function imageParameterFields(caps: ImageParameterConstraints | null) {
  if (!caps) return [];
  const fields: {
    key: ImageSelectParameter;
    label: MessageKey;
    values: string[];
  }[] = [
    {
      key: "n",
      label: "images.count",
      values:
        caps.max_results > 1
          ? Array.from({ length: Math.min(5, caps.max_results) }, (_, i) =>
              String(i + 1),
            )
          : [],
    },
    { key: "size", label: "images.size", values: caps.sizes },
    { key: "quality", label: "images.quality", values: caps.qualities },
    { key: "background", label: "images.background", values: caps.backgrounds },
    {
      key: "output_format",
      label: "images.outputFormat",
      values: caps.output_formats ?? [],
    },
    {
      key: "complexity",
      label: "images.complexity",
      values: caps.complexities ?? [],
    },
  ];
  return fields.filter((field) => field.values.length > 0);
}

export function validImageSize(
  size: string,
  limits: ImageSizeConstraints,
): boolean {
  if (!/^\d{2,4}x\d{2,4}$/.test(size)) return false;
  const [width = 0, height = 0] = size.split("x").map(Number);
  const pixels = width * height;
  return (
    limits.multiple === 16 &&
    limits.max_aspect_ratio > 0 &&
    limits.max_aspect_ratio <= 3 &&
    width >= Math.max(16, limits.min_edge) &&
    height >= Math.max(16, limits.min_edge) &&
    width <= Math.min(3840, limits.max_edge) &&
    height <= Math.min(3840, limits.max_edge) &&
    width % 16 === 0 &&
    height % 16 === 0 &&
    pixels >= Math.max(655360, limits.min_pixels) &&
    pixels <= Math.min(8294400, limits.max_pixels) &&
    width <= height * limits.max_aspect_ratio &&
    height <= width * limits.max_aspect_ratio
  );
}

export function imageParameterError(
  values: ImageParameters,
  caps: ImageParameterConstraints | null,
): MessageKey | null {
  if (!caps) return "images.invalidParameters";
  if (caps.contract && caps.contract !== IMAGE_PARAMETER_CONTRACT)
    return "images.invalidParameters";
  const n = values.n ?? 1;
  if (!Number.isInteger(n) || n < 1 || n > Math.min(5, caps.max_results))
    return "images.invalidParameters";
  for (const key of [
    "quality",
    "background",
    "output_format",
    "complexity",
  ] as const) {
    const value = values[key];
    if (
      value !== undefined &&
      !imageParameterFields(caps)
        .find((field) => field.key === key)
        ?.values.includes(value)
    )
      return "images.invalidParameters";
  }
  if (
    values.size !== undefined &&
    !caps.sizes.includes(values.size) &&
    !(caps.custom_size && validImageSize(values.size, caps.custom_size))
  )
    return "images.invalidSize";
  if (
    values.size &&
    values.size !== "auto" &&
    caps.contract === IMAGE_PARAMETER_CONTRACT &&
    !(caps.custom_size && validImageSize(values.size, caps.custom_size))
  )
    return "images.invalidSize";
  const extended =
    values.output_format !== undefined ||
    values.output_compression !== undefined ||
    values.complexity !== undefined ||
    values.quality === "xhigh" ||
    values.quality === "max";
  if (extended && caps.contract !== IMAGE_PARAMETER_CONTRACT)
    return "images.invalidParameters";
  if (values.background === "transparent" && values.output_format === "jpeg")
    return "images.transparentFormat";
  if (
    values.output_compression !== undefined &&
    (!caps.output_compression ||
      !["jpeg", "webp"].includes(values.output_format ?? "") ||
      !Number.isInteger(values.output_compression) ||
      values.output_compression < 0 ||
      values.output_compression > 100)
  )
    return "images.invalidCompression";
  return null;
}

export function supportedImageParameters(
  values: ImageParameters,
  caps: ImageParameterConstraints | null,
): ImageParameters {
  if (!caps) return {};
  const result = { ...values };
  for (const key of [
    "n",
    "quality",
    "size",
    "background",
    "output_format",
    "complexity",
  ] as const) {
    if (
      key === "size" &&
      values.size &&
      caps.custom_size &&
      validImageSize(values.size, caps.custom_size)
    )
      continue;
    if (
      !imageParameterFields(caps)
        .find((field) => field.key === key)
        ?.values.includes(String(values[key]))
    )
      delete result[key];
  }
  if (
    !caps.output_compression ||
    !["jpeg", "webp"].includes(result.output_format ?? "")
  )
    delete result.output_compression;
  return result;
}

const valueLabels: Partial<Record<string, MessageKey>> = {
  auto: "images.auto",
  low: "images.low",
  medium: "images.medium",
  high: "images.high",
  xhigh: "images.xhigh",
  max: "images.max",
  opaque: "images.opaque",
  transparent: "images.transparent",
};
export function imageParameterLabel(value: string): string {
  const key = valueLabels[value];
  return key
    ? t(key)
    : ["png", "jpeg", "webp"].includes(value)
      ? value.toUpperCase()
      : value;
}
export function imageParameterSummary(
  values: ImageParameters | ImageResultParameters,
): string {
  const parts: string[] = [];
  if ("n" in values && values.n !== undefined)
    parts.push(`${t("images.count")} ${values.n}`);
  for (const key of [
    "size",
    "quality",
    "background",
    "output_format",
  ] as const) {
    const value = values[key];
    if (value) parts.push(imageParameterLabel(value));
  }
  if ("complexity" in values && values.complexity)
    parts.push(
      `${t("images.complexity")} ${imageParameterLabel(values.complexity)}`,
    );
  if ("output_compression" in values && values.output_compression !== undefined)
    parts.push(`${t("images.compression")} ${values.output_compression}%`);
  return parts.length
    ? parts.join(" · ")
    : t(
        "observed_model" in values
          ? "images.parametersUnknown"
          : "images.default",
      );
}

export function imageFailureMessage(code: string): string {
  const keys: Partial<Record<string, MessageKey>> = {
    image_access_denied: "images.errorAccess",
    image_request_timeout: "images.errorTimeout",
    image_rate_limited: "images.errorRateLimit",
    invalid_image_request: "images.invalidParameters",
    unsupported_image_parameter: "images.invalidParameters",
    image_parameters_upgrade_required: "images.errorUpgrade",
    image_metering_upgrade_required: "images.errorUpgrade",
    model_unavailable: "images.errorModel",
    insufficient_balance: "images.errorBalance",
    api_key_spending_limit_exceeded: "images.errorBalance",
    invalid_image_response: "images.errorResponse",
    image_response_incomplete: "images.errorResponse",
    generation_response_failed: "images.errorResponse",
    result_unconfirmed: "images.cancelHint",
  };
  return t(keys[code] ?? "common.failed");
}
