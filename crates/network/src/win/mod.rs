//! Windows: адаптери (IP Helper), Wi-Fi (WLAN API), ICMP echo.
//!
//! Усі `unsafe` — виклики WinAPI з локальними буферами; кожен блок має коментар `// SAFETY:`.

mod wifi;

use crate::model::{classify, Adapter, NetworkInfo};
use crate::runner::EchoApi;
use crate::NetworkError;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GetBestInterfaceEx, GetIfEntry2, IcmpCloseHandle, IcmpCreateFile,
    IcmpSendEcho2, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST,
    ICMP_ECHO_REPLY, IP_ADAPTER_ADDRESSES_LH, IP_SUCCESS, MIB_IF_ROW2,
};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Networking::WinSock::{
    AF_INET, IN_ADDR, SOCKADDR, SOCKADDR_IN, SOCKET_ADDRESS,
};

/// Код типу інтерфейсу IANA: loopback.
const IF_TYPE_LOOPBACK: u32 = 24;

/// IPv4 із `SOCKET_ADDRESS` (інші сімейства — `None`).
fn ipv4(sa: &SOCKET_ADDRESS) -> Option<Ipv4Addr> {
    if sa.lpSockaddr.is_null() {
        return None;
    }
    // SAFETY: вказівник із відповіді GetAdaptersAddresses, дійсний, поки живе її буфер
    // (викликач тримає буфер); спершу читаємо лише сімейство.
    let family = unsafe { (*sa.lpSockaddr).sa_family };
    if family != AF_INET {
        return None;
    }
    // SAFETY: для AF_INET структура за вказівником — SOCKADDR_IN.
    let sin = unsafe { &*sa.lpSockaddr.cast::<SOCKADDR_IN>() };
    // SAFETY: читання поля об'єднання IN_ADDR (будь-яке подання — ті самі 4 байти).
    let raw = unsafe { sin.sin_addr.S_un.S_addr };
    Some(Ipv4Addr::from(u32::from_be(raw)))
}

/// Індекс інтерфейсу, через який іде маршрут в інтернет. Лише запит до таблиці маршрутів —
/// жодного пакета не надсилається.
fn internet_if_index() -> Option<u32> {
    let probe = SOCKADDR_IN {
        sin_family: AF_INET,
        sin_port: 0,
        sin_addr: IN_ADDR {
            S_un: windows::Win32::Networking::WinSock::IN_ADDR_0 {
                S_addr: u32::from(Ipv4Addr::new(8, 8, 8, 8)).to_be(),
            },
        },
        sin_zero: [0; 8],
    };
    let mut index = 0u32;
    // SAFETY: адреса — локальна SOCKADDR_IN, приведена до SOCKADDR (стандартний прийом WinSock).
    let rc = unsafe {
        GetBestInterfaceEx(
            (&probe as *const SOCKADDR_IN).cast::<SOCKADDR>(),
            &mut index,
        )
    };
    (rc == NO_ERROR.0).then_some(index)
}

/// Чи фізичний інтерфейс і його GUID (для WLAN API).
fn if_row(index: u32) -> Option<MIB_IF_ROW2> {
    let mut row = MIB_IF_ROW2 {
        InterfaceIndex: index,
        ..Default::default()
    };
    // SAFETY: локальна структура з заповненим індексом.
    (unsafe { GetIfEntry2(&mut row) } == NO_ERROR).then_some(row)
}

fn wide(p: windows::core::PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: рядок із відповіді GetAdaptersAddresses, нуль-термінований, буфер живий.
    unsafe { p.to_string() }.unwrap_or_default()
}

pub(crate) fn network_info() -> Result<NetworkInfo, NetworkError> {
    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    // Буфер вирівнюємо як u64 — так само вирівняна IP_ADAPTER_ADDRESSES_LH.
    let mut buf: Vec<u64> = vec![0; 16 * 1024 / 8];
    let mut rc = ERROR_BUFFER_OVERFLOW.0;
    for _ in 0..4 {
        let mut size = u32::try_from(buf.len() * 8).unwrap_or(u32::MAX);
        // SAFETY: буфер локальний, розмір у байтах передано; при нестачі функція повертає
        // потрібний розмір, і ми повторюємо з більшим буфером.
        rc = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_INET.0),
                flags,
                None,
                Some(buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>()),
                &mut size,
            )
        };
        if rc != ERROR_BUFFER_OVERFLOW.0 {
            break;
        }
        buf = vec![0; (size as usize).div_ceil(8) + 1];
    }
    if rc != NO_ERROR.0 {
        return Err(NetworkError::Os {
            api: "GetAdaptersAddresses",
            code: rc,
        });
    }

    let internet = internet_if_index();
    let mut adapters = Vec::new();
    let mut node = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    while !node.is_null() {
        // SAFETY: вузол зв'язного списку всередині нашого буфера; `Next` вказує на наступний
        // або нульовий.
        let a = unsafe { &*node };
        node = a.Next;
        if a.OperStatus != IfOperStatusUp
            || a.IfType == IF_TYPE_LOOPBACK
            || a.FirstUnicastAddress.is_null()
        {
            continue;
        }
        // SAFETY: читання поля об'єднання (довжина + індекс).
        let index = unsafe { a.Anonymous1.Anonymous.IfIndex };
        let row = if_row(index);
        let hardware = row
            .as_ref()
            .is_some_and(|r| r.InterfaceAndOperStatusFlags._bitfield & 1 != 0);
        let kind = classify(a.IfType, hardware);

        let mut gateway = None;
        let mut gw = a.FirstGatewayAddress;
        while !gw.is_null() && gateway.is_none() {
            // SAFETY: вузол списку шлюзів у нашому буфері.
            let g = unsafe { &*gw };
            gateway = ipv4(&g.Address);
            gw = g.Next;
        }
        let mut dns = Vec::new();
        let mut d = a.FirstDnsServerAddress;
        while !d.is_null() {
            // SAFETY: вузол списку DNS-серверів у нашому буфері.
            let n = unsafe { &*d };
            dns.extend(ipv4(&n.Address).map(|ip| ip.to_string()));
            d = n.Next;
        }
        let link = a.TransmitLinkSpeed.max(a.ReceiveLinkSpeed);
        let wifi = match (kind, &row) {
            (crate::model::LinkKind::Wifi, Some(r)) => wifi::query(&r.InterfaceGuid),
            _ => None,
        };
        adapters.push(Adapter {
            name: wide(a.FriendlyName),
            description: wide(a.Description),
            kind,
            // u64::MAX — «невідомо».
            link_mbps: (link > 0 && link != u64::MAX).then_some(link / 1_000_000),
            gateway: gateway.map(|g| g.to_string()),
            dns,
            carries_internet: internet == Some(index),
            wifi,
        });
    }
    Ok(NetworkInfo { adapters })
}

/// ICMP echo через `IcmpSendEcho2`. Час міряємо самі (монотонний годинник): поле
/// `RoundTripTime` у відповіді — цілі мілісекунди, замало для джитера в локальній мережі.
pub(crate) struct IcmpEcho;

/// Розмір корисного навантаження запиту (як у `ping` Windows).
const PAYLOAD: usize = 32;

impl EchoApi for IcmpEcho {
    fn echo(&self, target: Ipv4Addr, timeout: Duration) -> Option<f64> {
        // Окремий дескриптор на запит: потоки різних цілей не ділять стан.
        // SAFETY: без аргументів; закривається нижче.
        let handle = unsafe { IcmpCreateFile() }.ok()?;
        let payload = [0x61u8; PAYLOAD];
        // Відповідь + копія даних + місце для повідомлення про помилку ICMP (8 байт).
        let mut reply = vec![0u8; core::mem::size_of::<ICMP_ECHO_REPLY>() + PAYLOAD + 8 + 16];
        let started = Instant::now();
        // SAFETY: дескриптор щойно створено; адреса — u32 у мережевому порядку байтів; буфери
        // локальні, їхні розміри передано.
        let n = unsafe {
            IcmpSendEcho2(
                handle,
                None,
                None,
                None,
                u32::from_ne_bytes(target.octets()),
                payload.as_ptr().cast(),
                PAYLOAD as u16,
                None,
                reply.as_mut_ptr().cast(),
                reply.len() as u32,
                u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX),
            )
        };
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        // SAFETY: буфер щонайменше розміру ICMP_ECHO_REPLY; читаємо без вимог до вирівнювання.
        let status = (n > 0).then(|| unsafe {
            core::ptr::read_unaligned(reply.as_ptr().cast::<ICMP_ECHO_REPLY>()).Status
        });
        // SAFETY: дескриптор створено вище.
        let _ = unsafe { IcmpCloseHandle(handle) };
        (status == Some(IP_SUCCESS)).then_some(elapsed)
    }
}
