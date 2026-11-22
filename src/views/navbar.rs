use crate::api::ble_service::use_ble;
use crate::context::use_app_state;
use crate::Route;
use dioxus::prelude::*;
use dioxus_free_icons::icons::fa_brands_icons::FaBluetooth;
use dioxus_free_icons::icons::fa_solid_icons::{FaChartLine, FaSliders};
use dioxus_free_icons::Icon;

use super::TitleBar;

const APP_ICON: Asset = asset!("/assets/icon.svg");

#[component]
pub fn Navbar() -> Element {
    let mut app_state = use_app_state();
    let ble = use_ble();

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let current_id = app_state.connected_device_id.read().clone();
            if !current_id.is_empty()
                && !(app_state.is_connecting)()
                && !ble.is_connected(current_id.clone()).await
                && app_state.connected_device_id.read().as_str() == current_id
                && !(app_state.is_connecting)()
            {
                app_state.connected_device_id.set(String::new());
            }
        }
    });

    let theme = *app_state.theme.read();
    let is_connected = !app_state.connected_device_id.read().is_empty();

    // 根据主题选取颜色
    let root_bg = theme.pick("bg-[#0b111c]", "bg-[#f3f6f8]");
    let sidebar_bg = theme.pick("bg-[#111b2a]", "bg-white");
    let text_color = theme.pick("text-slate-100", "text-slate-800");
    let link_class  = theme.pick(
        "nav-link flex items-center mx-3 px-3 py-2.5 rounded-xl cursor-pointer no-underline text-slate-300 transition-all duration-200 text-sm hover:bg-white/10 hover:text-white",
        "nav-link flex items-center mx-3 px-3 py-2.5 rounded-xl cursor-pointer no-underline text-slate-600 transition-all duration-200 text-sm hover:bg-slate-100 hover:text-slate-900",
    );
    let active_class = theme.pick(
        "nav-link-active bg-[#1d3040] text-white relative before:content-[''] before:absolute before:left-0 before:top-2 before:bottom-2 before:w-1 before:rounded-full before:bg-[#b8f36b]",
        "nav-link-active bg-[#e9f2ed] text-slate-900 relative before:content-[''] before:absolute before:left-0 before:top-2 before:bottom-2 before:w-1 before:rounded-full before:bg-[#6ba83c]",
    );
    let main_bg = theme.pick("bg-[#0d1725]", "bg-[#f8fafb]");
    let sidebar_border = theme.pick("border-white/5", "border-slate-200");
    let brand_border = theme.pick("border-white/10", "border-slate-200");

    rsx! {
        div {
            class: "app-shell flex flex-col h-screen w-full {root_bg} {text_color} font-sans overflow-hidden",
            "data-theme": theme.pick("dark", "light"),
            // 自定义标题栏
            TitleBar {}

            // 主体区域：侧边栏 + 内容
            div { class: "flex flex-1 overflow-hidden",
                nav { class: "w-56 {sidebar_bg} flex flex-col py-4 shrink-0 border-r {sidebar_border}",
                    div { class: "px-5 pb-5 mb-3 border-b {brand_border}",
                        div { class: "flex items-center gap-3",
                            img { class: "w-10 h-10 rounded-xl shadow-lg shadow-cyan-500/10", src: APP_ICON, alt: "XSensor" }
                            div { class: "text-sm font-semibold tracking-wide", "XSensor" }
                        }
                        div { class: "mt-4 flex items-center gap-2 text-[11px] {theme.pick(\"text-slate-400\", \"text-slate-500\")}",
                            span { class: if is_connected { "status-dot status-dot-online" } else { "status-dot status-dot-offline" } }
                            span { if is_connected { "设备已连接" } else { "等待设备连接" } }
                        }
                    }
                    Link {
                        to: Route::Connection {},
                        class: "{link_class}",
                        active_class: "{active_class}",
                        Icon { width: 18, height: 18, icon: FaBluetooth, class: "mr-3" }
                        span { "连接" }
                    }

                    Link {
                        to: Route::Status {},
                        class: "{link_class}",
                        active_class: "{active_class}",
                        Icon { width: 18, height: 18, icon: FaChartLine, class: "mr-3" }
                        span { "状态" }
                    }

                    Link {
                        to: Route::Parameters {},
                        class: "{link_class}",
                        active_class: "{active_class}",
                        Icon { width: 18, height: 18, icon: FaSliders, class: "mr-3" }
                        span { "调参" }
                    }
                }

                main { class: "app-scroll flex-1 overflow-y-auto {main_bg} px-5 py-5",
                    Outlet::<Route> {}
                }
            }
        }
    }
}
