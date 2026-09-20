# wrapper-ble-esp32c3mini

[![Crates.io](https://img.shields.io/crates/v/wrapper-ble-esp32c3mini.svg)](https://crates.io/crates/wrapper-ble-esp32c3mini)
[![docs.rs](https://img.shields.io/docsrs/wrapper-ble-esp32c3mini)](https://docs.rs/wrapper-ble-esp32c3mini)
[![License: GPL-2.0-or-later](https://img.shields.io/badge/license-GPL--2.0--or--later-blue.svg)](./LICENSE)
[![Downloads](https://img.shields.io/crates/d/wrapper-ble-esp32c3mini.svg)](https://crates.io/crates/wrapper-ble-esp32c3mini)


Bibliothèque `no_std` qui encapsule l'initialisation d'un périphérique BLE
(*GATT peripheral*) sur ESP32-C3, au-dessus de `esp-radio`, `bt-hci` et
`trouble-host`.

Elle expose :

- `BleController` : alias vers le contrôleur HCI externe branché sur le
  `BleConnector` de `esp-radio` ;
- `BleServer` / `DisplayService` : un serveur GATT minimal avec deux
  caractéristiques (`command` en écriture, `status` en lecture/notification) ;
- `BleSystem::init(...)` : construit le contrôleur, la pile hôte
  `trouble-host` et le serveur GATT, et te rend `runner`, `peripheral` et
  `server` prêts à l'emploi ;
- `run_ble_runner(...)`: tâche à *spawner* qui fait tourner la boucle
  d'évènements de la pile hôte en continu.

## Installation

```toml
[dependencies]
wrapper-ble-esp32c3mini = "0.2"
```

## Historique des versions

- **0.2.0** retire la dépendance directe à `esp-println` (remplacée par
  la façade `log`, voir « Journalisation » plus bas). La 0.1.0 pouvait
  casser la compilation d'un binaire consommateur qui fixait lui-même une
  version d'`esp-println` différente, celui-ci ayant `links = "esp-println"`
  (une seule version autorisée dans tout l'arbre de dépendances).
- **0.1.0** première version.

## Pourquoi ces versions précises de dépendances ?

Le `Cargo.toml` fige volontairement certaines versions plutôt que de
prendre les toutes dernières de chaque crate, parce qu'elles ne sont **pas**
toutes inter-compatibles au moment de la rédaction :

| Crate          | Version retenue | Raison                                                                 |
|----------------|------------------|-------------------------------------------------------------------------|
| `esp-radio`    | `0.18.0`         | dernière stable                                                         |
| `esp-hal`      | `1.1.2`          | `esp-radio 0.18.0` exige `esp-hal ~1.1.0-rc.0` → branche `1.1.x` uniquement, pas `1.2.x` |
| `bt-hci`       | `0.8.1`          | `esp-radio 0.18.0` exige `bt-hci ^0.8.0`                                 |
| `trouble-host` | `0.6.0`          | seule branche de `trouble-host` qui dépend elle-même de `bt-hci ^0.8` (les versions `0.7`/`0.8` sont passées à `bt-hci ^0.9`/`^0.10`) |
| `heapless`     | `0.9`            | exigé par `trouble-host 0.6.0`                                          |
| `embassy-sync` | `0.7.2`          | dépendance directe requise : l'expansion de `#[gatt_server]`/`#[gatt_service]` par `trouble-host-macros 0.4.0` (la version réellement résolue par `trouble-host 0.6.0`, qui demande `^0.4.0`) référence `embassy_sync::...` sans passer par un ré-export de `trouble_host` |



## Journalisation

`run_ble_runner` journalise ses erreurs via la façade [`log`](https://docs.rs/log)
plutôt que via `esp-println` directement. `esp-println` déclare
`links = "esp-println"` (une seule version possible dans tout l'arbre de
dépendances) **et** exige qu'exactement une feature de sortie soit active
(`auto`, `uart`, `jtag-serial` ou `no-op`) deux contraintes qu'une
bibliothèque ne doit pas trancher à la place du binaire final, au risque
d'entrer en conflit avec son propre choix de version/feature. Le binaire
final reste libre d'utiliser `esp-println` (avec sa feature `log-04`),
`defmt`, ou tout autre backend `log`.

## Mémoire statique

`trouble-host` attend que ses ressources internes (`HostResources`) et la
`Stack` elle-même vivent en `'static`. `BleSystem::init` utilise donc
`static_cell::StaticCell` pour les promouvoir, plutôt que de les garder en
variables locales (qui ne compileraient pas avec les bornes de durée de vie
attendues par `Runner<'static, ...>` / `Peripheral<'static, ...>`).

## Durées de vie (`'static`)

Deux autres points de la signature sont volontairement fixés à `'static` :

- **`BleServer<'static>`** :`#[gatt_server]` génère un type portant un
  paramètre de durée de vie (`BleServer<'values>`), hérité de la chaîne de
  caractères passée à `PeripheralConfig::name`. Comme on lui passe toujours
  un `&'static str` (le nom de l'appareil est un littéral), cette durée de
  vie vaut systématiquement `'static` ici.
- **`bt_peripheral: esp_hal::peripherals::BT<'static>`** :`esp_hal::peripherals::BT`
  porte lui aussi un paramètre de durée de vie. Sans l'annoter explicitement
  dans la signature de `BleSystem::init`, le compilateur lui assigne une
  durée de vie anonyme propre à l'appel, incompatible avec
  `BleController = ExternalController<BleConnector<'static>, 20>`. Un
  `Peripherals` obtenu normalement via `esp_hal::init(...)` a des champs
  `'static` par défaut, donc `peripherals.BT` s'y prête directement.

## Exemple minimal

```rust,ignore
#![no_std]
#![no_main]

use wrapper_ble_esp32c3mini::{BleSystem, run_ble_runner};

#[esp_hal_embassy::main]
async fn main(spawner: embassy_executor::Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let ble = BleSystem::init(peripherals.BT);
    spawner.spawn(ble_runner_task(ble.runner)).unwrap();

    // `ble.peripheral` : publicité BLE + acceptation de connexions
    // `ble.server`     : lecture/écriture des caractéristiques GATT
}

#[embassy_executor::task]
async fn ble_runner_task(
    runner: trouble_host::Runner<'static, wrapper_ble_esp32c3mini::BleController, trouble_host::DefaultPacketPool>,
) {
    run_ble_runner(runner).await;
}
```

La configuration de l'exécuteur Embassy (via `esp-hal-embassy`) reste à la
charge du binaire final ; cette bibliothèque ne fait que fournir les
futures et la structure GATT.

## Licence

GPL-2.0-or-later — voir [`LICENSE`](./LICENSE).