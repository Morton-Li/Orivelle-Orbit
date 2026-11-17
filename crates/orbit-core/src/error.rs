use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PublicError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_parameters: Vec<String>,
}
impl PublicError {
    pub fn new(code: &str) -> Self {
        let message = match code {
            "vault_unavailable" => "系统安全存储不可用或访问被拒绝。",
            "credential_missing" => "所需凭据不可用，请在 Orbit 重新登录。",
            "binding_disabled" => "此客户端连接已停用。",
            "connection_conflict" => "连接已经改变，请重新预览。",
            "api_key_management_unavailable" => "Panel 尚不支持此版本的密钥管理，请先更新服务端。",
            "api_key_authorization_required" => "请重新连接账户，授权 Orbit 管理 API Key。",
            "api_key_operation_pending" => "上次密钥操作的结果尚待确认，请先核对该次操作。",
            "api_key_operation_conflict" => "该次密钥操作的内容已改变，请刷新后重试。",
            "api_key_operation_missing" => "没有待核对的密钥操作。",
            "api_key_operation_expired" => "旧操作已过期，请先核对当前密钥列表，再发起新的操作。",
            "api_key_changed" | "orbit_legacy_key_changed" => "密钥信息已经变化，请刷新后重试。",
            "api_key_unavailable" | "orbit_credential_unavailable" => {
                "这把密钥暂不可用，请刷新或选择其他密钥。"
            }
            "api_key_expired" => "请先更新密钥有效期，再进行轮换。",
            "invalid_api_key_request" => "请检查密钥名称、有效期、速度策略与限额设置。",
            "panel_origin_not_allowed" => "此版本仅连接 Orivelle 官方服务，请重新登录。",
            "configuration_conflict" => "配置已被其他程序修改，需协调冲突。",
            "schema_unverified" => "Claude 自动写入合同尚未启用；诊断候选仅供核查。",
            "integration_authorization_required" => "此系统集成尚未取得测试授权。",
            "result_unconfirmed" => "请求已发送，结果待确认。请勿自动重新生成。",
            "image_request_timeout" => "等待图片结果超时，请先核对任务状态，勿自动重新生成。",
            "image_parameters_unavailable" => "当前服务尚未配置本次所需的高级图片功能。",
            "image_api_pricing_unavailable" => "高级图片销售价格尚未配置完整，请联系管理员。",
            "image_limit_exceeded" => "图片、像素或完整请求超过资源上限。",
            "image_access_denied" => "当前账户无权使用此图片模型或操作。",
            "image_rate_limited" => "图片请求已达到额度或频率限制，请稍后查看额度状态。",
            "invalid_image_request" | "unsupported_image_parameter" => {
                "当前模型不支持所选图片参数或参数组合。"
            }
            "invalid_image_material" => "蒙版或标注须为与第一张参考图同尺寸的 PNG 图片。",
            "image_parameters_upgrade_required"
            | "image_metering_upgrade_required"
            | "image_routing_upgrade_required" => "图片服务需要升级以支持本次请求，请联系管理员。",
            "model_unavailable" => "当前图片模型不可用，请刷新模型列表。",
            "insufficient_balance" | "api_key_spending_limit_exceeded" => {
                "可用余额或消费额度不足。"
            }
            "invalid_image_response" | "image_response_incomplete" => {
                "图片响应无效或不完整，请先核对任务状态。"
            }
            "generation_response_failed" => "图片服务未能完成请求，请核对任务状态。",
            "invalid_image" => "图片格式或内容无效。",
            "unsafe_image_content" => "SVG 含有无法安全预览的脚本或外部资源。",
            "unsupported_image_input" => "参考图仅支持 PNG、JPEG 或 WebP；请先转换图片格式。",
            "local_save_failed" => "结果已收到，本地保存失败；仅重试本地保存。",
            "update_disabled" => "当前构建没有正式更新信任配置。",
            "update_busy" => "活动任务或凭据 helper 尚未结束，更新已暂停。",
            "update_request_timeout" => "更新请求超时，请检查网络后重试。",
            "update_request_failed" => "更新请求失败，请检查网络后重试。",
            "update_download_cancelled" => "已取消更新下载，可随时重新下载。",
            _ => "操作无法完成，请检查连接或诊断记录。",
        };
        Self {
            code: code.into(),
            message: message.into(),
            unsupported_parameters: vec![],
            retryable: false,
        }
    }
}
impl std::fmt::Display for PublicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.code)
    }
}
impl std::error::Error for PublicError {}
pub type Result<T> = std::result::Result<T, PublicError>;
pub fn fail(code: &str) -> PublicError {
    PublicError::new(code)
}
