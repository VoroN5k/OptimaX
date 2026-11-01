//! До якого пристрою належить драйвер — за іменем модуля ядра (щоб порада була про пристрій,
//! а не лише про файл `.sys`).

/// Вид пристрою драйвера.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverKind {
    /// Відеокарта й графічна підсистема Windows (переривання відеокарти йдуть через `dxgkrnl`).
    Gpu,
    /// Мережа: Ethernet, Wi-Fi, мережевий стек.
    Network,
    /// Накопичувачі та контролери дисків.
    Storage,
    /// Звук.
    Audio,
    /// USB і пристрої введення.
    Usb,
    /// Керування живленням процесора та ACPI.
    Power,
    /// Фреймворк драйверів KMDF: конкретний пристрій за ним не видно.
    Framework,
    /// Ядро Windows (таймери, планувальник).
    System,
    /// Інше.
    Other,
}

impl DriverKind {
    /// Ключ i18n назви виду.
    pub fn name_key(self) -> &'static str {
        match self {
            Self::Gpu => "diag.driver_kind.gpu",
            Self::Network => "diag.driver_kind.network",
            Self::Storage => "diag.driver_kind.storage",
            Self::Audio => "diag.driver_kind.audio",
            Self::Usb => "diag.driver_kind.usb",
            Self::Power => "diag.driver_kind.power",
            Self::Framework => "diag.driver_kind.framework",
            Self::System => "diag.driver_kind.system",
            Self::Other => "diag.driver_kind.other",
        }
    }

    /// Ключ i18n поради (з аргументом `{name}`).
    pub fn advice_key(self) -> &'static str {
        match self {
            Self::Gpu => "diag.advice.dpc_gpu",
            Self::Network => "diag.advice.dpc_network",
            Self::Storage => "diag.advice.dpc_storage",
            Self::Audio => "diag.advice.dpc_audio",
            Self::Usb => "diag.advice.dpc_usb",
            Self::Power => "diag.advice.dpc_power",
            Self::Framework => "diag.advice.dpc_framework",
            Self::System => "diag.advice.dpc_system",
            Self::Other => "diag.advice.dpc_other",
        }
    }
}

const GPU: &[&str] = &[
    "nvlddmkm",
    "dxgkrnl",
    "dxgmms",
    "amdkmdag",
    "atikmdag",
    "atikmpag",
    "amdkmpfd",
    "igdkmd",
    "basicdisplay",
];
const NETWORK: &[&str] = &[
    "ndis",
    "tcpip",
    "netio",
    "netwtw",
    "netwbw",
    "netwsw",
    "netadaptercx",
    "rt640",
    "rtwlan",
    "rtux64",
    "rtcx21",
    "e1d",
    "e1r",
    "e2f",
    "ixgbe",
    "iwl",
    "athw",
    "qcamain",
    "bcmwl",
    "wdiwifi",
    "vwifibus",
    "nwifi",
    "mrvlpcie",
];
const STORAGE: &[&str] = &[
    "storport", "stornvme", "storahci", "iastor", "nvme", "disk.sys", "classpnp", "ataport",
    "secnvme", "amdsata", "amdxata",
];
const AUDIO: &[&str] = &[
    "hdaudbus",
    "hdaudio",
    "portcls",
    "rtkvhd",
    "nvhda",
    "intcaudiobus",
    "intcsst",
];
const USB: &[&str] = &[
    "usbxhci", "ucx01000", "usbport", "usbhub", "usbccgp", "hidclass", "hidusb", "iusb3", "amdxhc",
];
const POWER: &[&str] = &["acpi.sys", "intelppm", "amdppm", "processr"];
const SYSTEM: &[&str] = &["ntoskrnl", "ntkrnl", "hal.dll"];

/// Вид пристрою за іменем модуля (без урахування регістру).
pub fn driver_kind(name: &str) -> DriverKind {
    let n = name.to_ascii_lowercase();
    let has = |list: &[&str]| list.iter().any(|p| n.starts_with(p));
    if has(GPU) {
        DriverKind::Gpu
    } else if has(NETWORK) {
        DriverKind::Network
    } else if has(STORAGE) {
        DriverKind::Storage
    } else if has(AUDIO) {
        DriverKind::Audio
    } else if has(USB) {
        DriverKind::Usb
    } else if has(POWER) {
        DriverKind::Power
    } else if n.starts_with("wdf01000") {
        DriverKind::Framework
    } else if has(SYSTEM) {
        DriverKind::System
    } else {
        DriverKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_drivers_map_to_devices() {
        assert_eq!(driver_kind("nvlddmkm.sys"), DriverKind::Gpu);
        assert_eq!(driver_kind("dxgkrnl.sys"), DriverKind::Gpu);
        assert_eq!(driver_kind("Netwtw10.sys"), DriverKind::Network);
        assert_eq!(driver_kind("tcpip.sys"), DriverKind::Network);
        assert_eq!(driver_kind("stornvme.sys"), DriverKind::Storage);
        assert_eq!(driver_kind("HDAudBus.sys"), DriverKind::Audio);
        assert_eq!(driver_kind("USBXHCI.SYS"), DriverKind::Usb);
        assert_eq!(driver_kind("ACPI.sys"), DriverKind::Power);
        assert_eq!(driver_kind("Wdf01000.sys"), DriverKind::Framework);
        assert_eq!(driver_kind("ntoskrnl.exe"), DriverKind::System);
        assert_eq!(driver_kind("someoem.sys"), DriverKind::Other);
    }
}
