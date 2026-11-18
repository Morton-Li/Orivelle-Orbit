import type { ConfigPreview, ConnectionView, ModelView } from "./contracts";

export function matchesReviewConnection(
  preview: ConfigPreview,
  connection: ConnectionView | null | undefined,
): boolean {
  if (!connection || preview.operation !== "apply") return false;
  const context = preview.context;
  return (
    connection.id === context.connection_id &&
    connection.account_id === context.account_id &&
    connection.revision === context.connection_revision &&
    connection.credential_revision === context.credential_revision &&
    connection.node_id === context.node_id
  );
}

export function matchesReviewModel(
  preview: ConfigPreview,
  models: ModelView[],
): boolean {
  return (
    preview.operation === "apply" &&
    models.some(
      (model) =>
        model.id === preview.model_id &&
        model.revision === preview.context.model_revision,
    )
  );
}
