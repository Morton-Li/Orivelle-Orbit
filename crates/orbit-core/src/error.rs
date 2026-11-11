use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PublicError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}
impl PublicError {
    pub fn new(code: &str) -> Self {
        let message = match code {
            "vault_unavailable" => "系统安全存储不可用或访问被拒绝。",
            "credential_missing" => "所需凭据不可用，请在 Orbit 重新登录。",
            "binding_disabled" => "此客户端连接已停用。",
            "connection_conflict" => "连接已经改变，请重新预览。",
            "panel_origin_not_allowed" => "此版本仅连接 Orivelle 官方服务，请重新登录。",
            "configuration_conflict" => "配置已被其他程序修改，需协调冲突。",
            "schema_unverified" => "Claude 自动写入合同尚未启用；诊断候选仅供核查。",
            "integration_authorization_required" => "此系统集成尚未取得测试授权。",
            "result_unconfirmed" => "请求已发送，结果待确认。请勿自动重新生成。",
            "image_request_timeout" => "等待图片结果超时，请先核对任务状态，勿自动重新生成。",
            "image_limit_exceeded" => "图片、像素或完整请求超过资源上限。",
            "image_access_denied" => "当前账户无权使用此图片模型或操作。",
            "image_rate_limited" => "图片请求已达到额度或频率限制，请稍后查看额度状态。",
            "invalid_image_request" | "unsupported_image_parameter" => {
                "当前模型不支持所选图片参数或参数组合。"
            }
            "image_parameters_upgrade_required" | "image_metering_upgrade_required" => {
                "图片服务需要升级以支持本次请求，请联系管理员。"
            }
            "model_unavailable" => "当前图片模型不可用，请刷新模型列表。",
            "insufficient_balance" | "api_key_spending_limit_exceeded" => {
                "可用余额或消费额度不足。"
            }
            "invalid_image_response" | "image_response_incomplete" => {
                "图片响应无效或不完整，请先核对任务状态。"
            }
            "generation_response_failed" => "图片服务未能完成请求，请核对任务状态。",
            "invalid_image" => "图片格式或内容无效。",
            "local_save_failed" => "结果已收到，本地保存失败；仅重试本地保存。",
            "update_disabled" => "当前构建没有正式更新信任配置。",
            "update_busy" => "活动任务或凭据 helper 尚未结束，更新已暂停。",
            _ => "操作无法完成，请检查连接或诊断记录。",
        };
        Self {
            code: code.into(),
            message: message.into(),
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
