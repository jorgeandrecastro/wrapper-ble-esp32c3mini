//! # wrapper-ble-esp32c3mini
//!
//! Bibliothèque `no_std` encapsulant l'initialisation d'un périphérique BLE
//! (rôle *GATT peripheral*) sur ESP32-C3, en s'appuyant sur :
//!
//! - [`esp-radio`](https://docs.rs/esp-radio) pour le contrôleur HCI matériel
//!   ([`BleConnector`]),
//! - [`bt-hci`](https://docs.rs/bt-hci) et son [`ExternalController`] pour
//!   exposer ce contrôleur au format attendu par la pile hôte,
//! - [`trouble-host`](https://docs.rs/trouble-host) pour la pile hôte BLE
//!   (GAP, GATT, L2CAP) et l'exécution asynchrone via Embassy.
//!
//! ## Vue d'ensemble
//!
//! Le point d'entrée est [`BleSystem::init`], qui construit :
//! 1. le contrôleur BLE matériel ([`BleController`]),
//! 2. la pile hôte `trouble-host` configurée en rôle périphérique,
//! 3. un serveur GATT [`BleServer`] exposant le service [`DisplayService`].
//!
//! Le `[`Runner`](trouble_host::Runner)` renvoyé doit ensuite être piloté en
//! continu par la tâche [`run_ble_runner`], typiquement *spawnée* sur
//! l'exécuteur Embassy du binaire final.
//!
//! ## Exemple d'utilisation (squelette)
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! use wrapper_ble_esp32c3mini::{BleSystem, run_ble_runner};
//!
//! #[esp_hal_embassy::main]
//! async fn main(spawner: embassy_executor::Spawner) {
//!     let peripherals = esp_hal::init(esp_hal::Config::default());
//!
//!     let BleSystem { runner, peripheral, server } = BleSystem::init(peripherals.BT);
//!
//!     spawner.spawn(ble_runner_task(runner)).unwrap();
//!
//!     // `peripheral` sert ensuite à publicité + acceptation de connexions,
//!     // `server` expose les caractéristiques `command` / `status`.
//! }
//!
//! #[embassy_executor::task]
//! async fn ble_runner_task(runner: trouble_host::Runner<'static, wrapper_ble_esp32c3mini::BleController, trouble_host::DefaultPacketPool>) {
//!     wrapper_ble_esp32c3mini::run_ble_runner(runner).await;
//! }
//! ```
//!
//! ## Compatibilité des versions
//!
//! Les versions figées dans `Cargo.toml` ne sont pas arbitraires : elles
//! forment le seul jeu de versions récentes qui se lient toutes ensemble au
//! moment de la rédaction de ce crate.
//!
//! - `esp-radio = "0.18.0"` exige `esp-hal` dans la plage `~1.1.0-rc.0`
//!   (donc `esp-hal = "1.1.2"`, **pas** la branche `1.2.x`) et `bt-hci`
//!   dans la plage `^0.8.0`.
//! - `bt-hci = "0.8.1"` est donc la version retenue, ce qui impose à son
//!   tour `trouble-host = "0.6.0"` (la seule branche de `trouble-host` qui
//!   dépend elle-même de `bt-hci ^0.8`, les versions `0.7`/`0.8` étant
//!   passées à `bt-hci ^0.9`/`^0.10`).
//!
//! Si, à l'avenir, `esp-radio` publie une version qui suit `esp-hal 1.2.x`
//! et un `bt-hci` plus récent, il faudra remonter `trouble-host` en même
//! temps que `bt-hci`, pas séparément.

#![no_std]

use bt_hci::controller::ExternalController;
use embassy_time::{Duration, Timer};
use esp_radio::ble::controller::BleConnector;
use static_cell::StaticCell;
use trouble_host::prelude::*;

/// Taille (en octets) du buffer HCI utilisé par le contrôleur externe.
///
/// `20` correspond à la taille de payload HCI ACL par défaut utilisée dans
/// les exemples `esp-radio` / `trouble-host`. Augmenter cette valeur permet
/// de négocier un MTU L2CAP plus grand, au prix de plus de RAM statique.
pub type BleController = ExternalController<BleConnector<'static>, 20>;

/// Serveur GATT exposé par ce périphérique BLE.
///
/// Contient un unique service, [`DisplayService`], monté à la construction
/// via [`BleServer::new_with_config`].
#[gatt_server]
pub struct BleServer {
    /// Service GATT exposant les caractéristiques `command` et `status`.
    pub display: DisplayService,
}

/// Service GATT « Display », de type Nordic UART Service (UUID `6e400001…`).
///
/// - `command` : caractéristique en écriture, utilisée par le client BLE
///   pour envoyer des ordres à l'appareil (jusqu'à 16 octets).
/// - `status`  : caractéristique en lecture + notification, utilisée par
///   l'appareil pour signaler son état au client (jusqu'à 16 octets).
#[gatt_service(uuid = "6e400001-b5a3-f393-e0a9-e50e24dcca9e")]
pub struct DisplayService {
    /// Commande envoyée par le client (écriture avec ou sans réponse).
    #[characteristic(uuid = "6e400002-b5a3-f393-e0a9-e50e24dcca9e", write, write_without_response)]
    pub command: heapless::Vec<u8, 16>,

    /// État courant de l'appareil, lisible et notifiable.
    #[characteristic(uuid = "6e400003-b5a3-f393-e0a9-e50e24dcca9e", read, notify)]
    pub status: heapless::Vec<u8, 16>,
}

/// Regroupe les trois composants nécessaires au fonctionnement de la pile
/// BLE : le *runner* de la pile hôte, le rôle périphérique et le serveur
/// GATT applicatif.
///
/// Ces trois éléments sont volontairement scindés (plutôt que gardés dans
/// une seule struct opaque) car `trouble-host` attend qu'ils soient
/// utilisés indépendamment : le `runner` est piloté en tâche de fond
/// ([`run_ble_runner`]), tandis que `peripheral` sert à publier des
/// annonces BLE et `server` à répondre aux lectures/écritures GATT.
pub struct BleSystem {
    /// Boucle d'évènements de la pile hôte BLE ; doit tourner en continu.
    pub runner: Runner<'static, BleController, DefaultPacketPool>,
    /// Rôle périphérique : publicité BLE et acceptation de connexions.
    pub peripheral: Peripheral<'static, BleController, DefaultPacketPool>,
    /// Serveur GATT applicatif (voir [`BleServer`]).
    ///
    /// `#[gatt_server]` génère un type portant un paramètre de durée de vie
    /// (`BleServer<'values>`), hérité de la chaîne de caractères passée à
    /// `PeripheralConfig::name`. Comme on lui passe toujours un `&'static
    /// str` (le nom de l'appareil est un littéral), cette durée de vie vaut
    /// systématiquement `'static` ici.
    pub server: BleServer<'static>,
}

impl BleSystem {
    /// Initialise le contrôleur BLE matériel, la pile hôte `trouble-host`
    /// et le serveur GATT, et retourne les trois composants prêts à
    /// l'emploi dans un [`BleSystem`].
    ///
    /// # Mémoire statique
    ///
    /// `trouble-host` exige que ses ressources internes ([`HostResources`])
    /// ainsi que la [`Stack`] elle-même vivent aussi longtemps que le
    /// `runner`/`peripheral` retournés (borne `'static`). Comme ce ne sont
    /// pas des variables globales au sens Rust, elles sont promues en
    /// `'static` via [`StaticCell`], le motif standard dans l'écosystème
    /// Embassy pour ce cas de figure.
    ///
    /// # Panics
    ///
    /// Cette fonction panique si :
    /// - l'initialisation du [`BleConnector`] échoue (matériel radio
    ///   indisponible ou mal configuré),
    /// - la création du serveur GATT échoue (par exemple si le nom du
    ///   périphérique dépasse la longueur maximale autorisée par le GAP,
    ///   22 octets).
    ///
    /// Ces deux échecs sont considérés comme irrécupérables à ce stade de
    /// l'initialisation et ne peuvent pas être corrigés à l'exécution ;
    /// c'est pourquoi ils sont remontés par `panic!` plutôt que par un
    /// `Result`. Si tu préfères une gestion d'erreur récupérable, remplace
    /// les `.expect(...)` par une propagation via `Result`.
    pub fn init(bt_peripheral: esp_hal::peripherals::BT<'static>) -> Self {
        let transport = BleConnector::new(bt_peripheral, esp_radio::ble::Config::default())
            .expect("Erreur init BleConnector");
        let controller: BleController = ExternalController::new(transport);

        // Ressources internes de la pile hôte (files d'attente L2CAP, etc.)
        // Générique <PacketPool, CONNS, CHANNELS> : 1 connexion simultanée,
        // 1 canal L2CAP. Augmente ces constantes si tu as besoin de gérer
        // plusieurs connexions ou canaux en parallèle.
        static RESOURCES: StaticCell<HostResources<DefaultPacketPool, 1, 1>> = StaticCell::new();
        let resources = RESOURCES.init(HostResources::new());

        // La `Stack` doit elle aussi être promue `'static` : `build()`
        // attend `&'stack self` avec `'stack` égal au paramètre de durée de
        // vie de la `Stack`, lui-même hérité de `resources`.
        static STACK: StaticCell<Stack<'static, BleController, DefaultPacketPool>> =
            StaticCell::new();
        let stack = STACK.init(
            trouble_host::new(controller, resources)
                // Adresse aléatoire statique (bit de poids fort à 1) ; à
                // remplacer par une adresse propre à chaque appareil en
                // production (ex. dérivée de l'identifiant matériel unique).
                .set_random_address(Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff])),
        );

        let Host {
            peripheral,
            runner,
            ..
        } = stack.build();

        let server = BleServer::new_with_config(GapConfig::Peripheral(PeripheralConfig {
            name: "ESP32C3-Mini",
            appearance: &appearance::UNKNOWN,
        }))
        .expect("Erreur création serveur GATT");

        Self {
            runner,
            peripheral,
            server,
        }
    }
}

/// Fait tourner indéfiniment la boucle d'évènements de la pile hôte BLE.
///
/// À *spawner* sur l'exécuteur Embassy du binaire final (voir l'exemple du
/// module). En cas d'erreur du runner (déconnexion inattendue du
/// contrôleur, par exemple), l'erreur est journalisée via [`esp_println`]
/// et la boucle retente après une seconde plutôt que de paniquer, afin de
/// ne pas interrompre le reste de l'application.
pub async fn run_ble_runner(mut runner: Runner<'_, BleController, DefaultPacketPool>) {
    loop {
        if let Err(e) = runner.run().await {
            esp_println::println!("Erreur du runner Bluetooth : {:?}", e);
            Timer::after(Duration::from_secs(1)).await;
        }
    }
}