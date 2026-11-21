use dioxus::prelude::*;

/// 全屏加载遮罩（绝对定位，覆盖父容器）
#[component]
pub fn LoadingOverlay(message: &'static str) -> Element {
    rsx! {
        div { class: "absolute inset-0 z-50 flex items-center justify-center bg-[#0b111c]/80 backdrop-blur-sm",
            div { class: "flex flex-col items-center gap-3",
                span { class: "animate-spin inline-block w-9 h-9 border-[3px] border-[#b8f36b] border-t-transparent rounded-full" }
                span { class: "text-sm text-slate-200 font-medium", "{message}" }
            }
        }
    }
}
