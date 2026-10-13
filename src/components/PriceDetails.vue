<script setup lang="ts">
import type { ModelView, PriceView, PricingMeter } from "../shared/contracts";
import { formatDecimal } from "../shared/format";
import { t, type MessageKey } from "../shared/i18n";
const props = defineProps<{ model: ModelView }>();
const meters: Record<PricingMeter, MessageKey> = {
  input: "meter.input",
  cached_input: "meter.cachedInput",
  cache_write: "meter.cacheWrite",
  output: "meter.output",
  reasoning: "meter.reasoning",
  request: "meter.request",
  image: "meter.image",
  text_input: "meter.textInput",
  cached_text_input: "meter.cachedText",
  image_input: "meter.imageInput",
  cached_image_input: "meter.cachedImage",
  image_output: "meter.imageOutput",
};
function priceTitle(price: PriceView) {
  return t(
    price.source === "official"
      ? "images.priceOfficial"
      : price.source === "sales"
        ? "images.priceSales"
        : price.source === "operator_estimate"
          ? "images.priceEstimate"
          : "common.unknown",
  );
}
function prices() {
  return [props.model.sales_price, props.model.official_price].filter(
    (price): price is PriceView => price !== null,
  );
}
</script>
<template>
  <details class="price-details">
    <summary>
      {{ t("images.pricing")
      }}<span class="muted">{{
        t(
          model.image_billing_mode === "image_count"
            ? "images.legacyBilling"
            : model.image_billing_mode === "image_tokens_v1"
              ? "images.tokenBilling"
              : "common.unknown",
        )
      }}</span>
    </summary>
    <div v-for="(price, index) in prices()" :key="index" class="price-table">
      <h3>{{ priceTitle(price) }}</h3>
      <p
        v-if="!price.rates.length || price.status !== 'configured'"
        class="muted"
      >
        {{
          t(
            price.status === "unpriced_free"
              ? "images.unpricedFree"
              : "common.unknown",
          )
        }}
      </p>
      <div v-for="rate in price.rates" :key="rate.meter" class="price-rate">
        <span>{{ t(meters[rate.meter]) }}</span
        ><span
          >{{ price.currency ?? t("common.unknown") }}
          {{
            rate.unit_price === null
              ? t("common.unknown")
              : (formatDecimal(rate.unit_price) ?? t("common.unknown"))
          }}
          / {{ rate.unit_size.toLocaleString() }}</span
        >
      </div>
    </div>
    <p class="field-hint">{{ t("images.priceHint") }}</p>
  </details>
</template>
