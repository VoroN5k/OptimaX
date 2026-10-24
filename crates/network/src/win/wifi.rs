//! Wi-Fi через WLAN API: якість і рівень сигналу, діапазон, канал, стандарт.
//!
//! З Windows 11 24H2 назва мережі й параметри точки доступу (BSS) доступні класичним
//! програмам лише з дозволом на розташування. Без нього повертаємо те, що є, і позначаємо
//! `details_blocked` — інтерфейс пояснює, як увімкнути.

use crate::model::{band_and_channel, wifi_standard, WifiInfo};
use core::ffi::c_void;
use windows::core::GUID;
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, HANDLE};
use windows::Win32::NetworkManagement::WiFi::{
    dot11_BSS_type_infrastructure, wlan_intf_opcode_current_connection, wlan_intf_opcode_rssi,
    WlanCloseHandle, WlanFreeMemory, WlanGetNetworkBssList, WlanOpenHandle, WlanQueryInterface,
    WLAN_BSS_ENTRY, WLAN_BSS_LIST, WLAN_CONNECTION_ATTRIBUTES, WLAN_INTF_OPCODE,
};

/// Версія клієнта WLAN API для Windows Vista+.
const WLAN_CLIENT_VERSION: u32 = 2;

/// Дескриптор WLAN, що закривається в Drop.
struct Client(HANDLE);

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: дескриптор отримано з WlanOpenHandle.
        let _ = unsafe { WlanCloseHandle(self.0, None) };
    }
}

/// Пам'ять, виділена WLAN API, звільняється в Drop.
struct WlanMem(*mut c_void);

impl Drop for WlanMem {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: пам'ять видана функцією WLAN API.
            unsafe { WlanFreeMemory(self.0) };
        }
    }
}

fn query_opcode(client: &Client, guid: &GUID, opcode: WLAN_INTF_OPCODE) -> Result<WlanMem, u32> {
    let mut size = 0u32;
    let mut data: *mut c_void = core::ptr::null_mut();
    // SAFETY: дескриптор і GUID дійсні; вихідні параметри — локальні змінні.
    let rc =
        unsafe { WlanQueryInterface(client.0, guid, opcode, None, &mut size, &mut data, None) };
    let mem = WlanMem(data);
    if rc != 0 {
        return Err(rc);
    }
    Ok(mem)
}

/// Стан Wi-Fi-інтерфейсу з GUID `guid` (`None` — не Wi-Fi або не підключено).
pub(super) fn query(guid: &GUID) -> Option<WifiInfo> {
    let mut version = 0u32;
    let mut handle = HANDLE::default();
    // SAFETY: вихідні параметри — локальні змінні; дескриптор закриває Client.
    if unsafe { WlanOpenHandle(WLAN_CLIENT_VERSION, None, &mut version, &mut handle) } != 0 {
        return None;
    }
    let client = Client(handle);

    let conn = match query_opcode(&client, guid, wlan_intf_opcode_current_connection) {
        Ok(m) if !m.0.is_null() => m,
        Err(rc) if rc == ERROR_ACCESS_DENIED.0 => {
            return Some(WifiInfo {
                ssid: None,
                signal_pct: None,
                rssi_dbm: None,
                band: None,
                channel: None,
                standard: None,
                rx_mbps: 0,
                tx_mbps: 0,
                details_blocked: true,
            });
        }
        _ => return None,
    };
    // SAFETY: для цього коду WLAN API повертає WLAN_CONNECTION_ATTRIBUTES.
    let attrs = unsafe { &*conn.0.cast::<WLAN_CONNECTION_ATTRIBUTES>() };
    let assoc = &attrs.wlanAssociationAttributes;
    let ssid_len = (assoc.dot11Ssid.uSSIDLength as usize).min(assoc.dot11Ssid.ucSSID.len());
    let ssid = (ssid_len > 0)
        .then(|| String::from_utf8_lossy(&assoc.dot11Ssid.ucSSID[..ssid_len]).into_owned());

    let rssi = query_opcode(&client, guid, wlan_intf_opcode_rssi)
        .ok()
        .filter(|m| !m.0.is_null())
        // SAFETY: для коду rssi повертається один i32 (LONG).
        .map(|m| unsafe { *m.0.cast::<i32>() });

    // Частота поточної точки доступу (діапазон і канал, зокрема 6 ГГц) — зі списку BSS.
    let mut list: *mut WLAN_BSS_LIST = core::ptr::null_mut();
    // SAFETY: дескриптор, GUID і SSID дійсні; вихідний вказівник — локальний.
    let rc = unsafe {
        WlanGetNetworkBssList(
            client.0,
            guid,
            Some(&assoc.dot11Ssid),
            dot11_BSS_type_infrastructure,
            attrs.wlanSecurityAttributes.bSecurityEnabled.as_bool(),
            None,
            &mut list,
        )
    };
    let list_mem = WlanMem(list.cast());
    let mut entry_freq = None;
    if rc == 0 && !list.is_null() {
        // SAFETY: список виданий WLAN API; записи йдуть суцільним масивом довжиною dwNumberOfItems.
        let entries = unsafe {
            std::slice::from_raw_parts(
                (*list).wlanBssEntries.as_ptr(),
                (*list).dwNumberOfItems as usize,
            )
        };
        entry_freq = entries
            .iter()
            .find(|e: &&WLAN_BSS_ENTRY| e.dot11Bssid == assoc.dot11Bssid)
            .map(|e| (e.ulChCenterFrequency, e.lRssi));
    }
    drop(list_mem);
    let band_channel = entry_freq.and_then(|(f, _)| band_and_channel(f));

    Some(WifiInfo {
        ssid,
        signal_pct: Some(assoc.wlanSignalQuality.min(100)),
        rssi_dbm: rssi.or(entry_freq.map(|(_, r)| r)),
        band: band_channel.map(|(b, _)| b),
        channel: band_channel.map(|(_, c)| c),
        standard: wifi_standard(assoc.dot11PhyType.0).map(str::to_owned),
        rx_mbps: assoc.ulRxRate / 1000,
        tx_mbps: assoc.ulTxRate / 1000,
        details_blocked: rc == ERROR_ACCESS_DENIED.0,
    })
}
