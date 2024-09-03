use super::VmProvider;
use crate::{arch::Arch, steps};
use async_trait::async_trait;
use clap::Args;
use gcloud_sdk::google_rest_apis::compute_v1::{
    firewall::Direction,
    firewalls_api::{ComputePeriodFirewallsPeriodGetParams, ComputePeriodFirewallsPeriodInsertParams},
    instance::Status,
    instances_api::{
        ComputePeriodInstancesPeriodDeleteParams, ComputePeriodInstancesPeriodGetParams, ComputePeriodInstancesPeriodInsertParams,
        ComputePeriodInstancesPeriodListParams,
    },
    machine_types_api::ComputePeriodMachineTypesPeriodListParams,
    AccessConfig, AttachedDisk, AttachedDiskInitializeParams, Error, Firewall, FirewallAllowedInner, Instance, Metadata, MetadataItemsInner, NetworkInterface,
    Scheduling, Tags,
};
use rand::distributions::Alphanumeric;
use rand::Rng;
use std::{net::Ipv4Addr, str::FromStr as _};
use tokio::time::{sleep, Duration};

const INSTANCE_TAG: &str = "fleeting";
const INBOUND_FIREWALL_RULE_NAME: &str = "fleeting-allow-inbound";

/// Google Compute Engine
#[derive(Args, Clone)]
#[command(
    override_usage = color_print::cstr! {r#"<bold>fleeting</bold> <bold>gce</bold> [OPTIONS] [COMMAND]...

<bold><underline>Authentication:</underline></bold>
  - GOOGLE_APPLICATION_CREDENTIALS (pointing to JSON file)
  - gcloud auth application-default login
  - Metadata server, if running on GCE

<bold><underline>Setup:</underline></bold>
  - Create a project
  - Enable the Compute Engine API for it
  - Create a service account and download credentials JSON

<bold><underline>Limitations:</underline></bold>
While GCE instances will automatically stop, they will not be automatically
deleted. fleeting collects garbage at the beginning of the run, but you will
be left with a small number of stopped instances and will continue to pay for
their associated disks. Hopefully, this will be resolved in the future with
termination_time / max_run_duration, once GCE client libraries support it.

"#},)]
pub struct Gce {
    /// Project in which to create instances [required]
    #[arg(long)]
    project: String,

    #[arg(long, default_value = "us-central1-a")]
    zone: String,

    #[arg(long, default_value = "e2-micro")]
    machine_type: String,

    /// Disk size, in GiBs.
    #[arg(long)]
    disk: Option<usize>,
}

#[async_trait]
impl VmProvider for Gce {
    async fn spawn(&self, user_data: &str) -> anyhow::Result<Ipv4Addr> {
        let step = steps::start();
        log::info!("Loading Google Cloud configuration...");
        let google_rest_api = gcloud_sdk::GoogleRestApi::new().await?;
        let configuration = google_rest_api.create_google_compute_v1_config().await?;

        let step: _ = step.next();
        log::info!("Delete terminated fleeting instances...");
        {
            let instances = gcloud_sdk::google_rest_apis::compute_v1::instances_api::compute_instances_list(
                &configuration,
                ComputePeriodInstancesPeriodListParams {
                    project: self.project.to_owned(),
                    zone: self.zone.to_owned(),
                    filter: Some(r#"(name = "fleeting-*") AND (status = TERMINATED)"#.to_owned()),
                    ..Default::default()
                },
            )
            .await?
            .items
            .unwrap_or_default();

            for instance in &instances {
                let instance_name = instance.name.as_deref().unwrap();
                assert!(instance_name.starts_with("fleeting-"));
                gcloud_sdk::google_rest_apis::compute_v1::instances_api::compute_instances_delete(
                    &configuration,
                    ComputePeriodInstancesPeriodDeleteParams {
                        project: self.project.to_owned(),
                        zone: self.zone.to_owned(),
                        instance: instance_name.to_owned(),
                        ..Default::default()
                    },
                )
                .await?;
            }
            log::info!("{} deleted", instances.len());
        }

        let step: _ = step.next();
        log::info!("Looking up machine type...");
        let source_image = {
            // Problem 1: The client lib does not support the architecture field, but we can squeeze a string into a filter and see what matches
            let mut matched_archs = vec![];
            for google_arch in ["arm64", "x86_64"] {
                let num_matches = gcloud_sdk::google_rest_apis::compute_v1::machine_types_api::compute_machine_types_list(
                    &configuration,
                    ComputePeriodMachineTypesPeriodListParams {
                        project: self.project.to_owned(),
                        zone: self.zone.to_owned(),
                        filter: Some(format!("(name = {name}) AND (architecture = {google_arch})", name = self.machine_type)),
                        ..Default::default()
                    },
                )
                .await?
                .items
                .unwrap_or_default()
                .len();
                assert!(num_matches == 0 || num_matches == 1, "list returned {num_matches} matches");
                if num_matches == 1 {
                    matched_archs.push(Arch::from_str(google_arch).unwrap())
                }
            }
            log::debug!("{matched_archs:?}");

            // Problem 2: The API does not have an associated architecture for all machine types, e.g. e2-micro, so we have to assume
            let arch = match &*matched_archs {
                [] => Arch::Amd64, // assumed
                [arch] => *arch,
                x => panic!("multiple architecture filters matched: {x:?}"),
            };

            format!("projects/ubuntu-os-cloud/global/images/family/ubuntu-2404-lts-{}", arch.as_dpkg())
        };

        let step: _ = step.next();
        log::info!("Creating firewall rule if needed...");
        {
            let result = gcloud_sdk::google_rest_apis::compute_v1::firewalls_api::compute_firewalls_get(
                &configuration,
                ComputePeriodFirewallsPeriodGetParams {
                    project: self.project.to_owned(),
                    firewall: INBOUND_FIREWALL_RULE_NAME.to_owned(),
                    ..Default::default()
                },
            )
            .await;

            match result {
                Err(Error::ResponseError(content)) if content.status.as_u16() == 404 => {
                    gcloud_sdk::google_rest_apis::compute_v1::firewalls_api::compute_firewalls_insert(
                        &configuration,
                        ComputePeriodFirewallsPeriodInsertParams {
                            project: self.project.to_owned(),
                            firewall: Some(Firewall {
                                name: Some(INBOUND_FIREWALL_RULE_NAME.to_owned()),
                                target_tags: Some(vec![INSTANCE_TAG.to_owned()]),
                                direction: Some(Direction::Ingress),
                                allowed: Some(vec![
                                    FirewallAllowedInner { ip_protocol: Some("tcp".to_owned()), ..Default::default() },
                                    FirewallAllowedInner { ip_protocol: Some("udp".to_owned()), ..Default::default() },
                                    FirewallAllowedInner { ip_protocol: Some("icmp".to_owned()), ..Default::default() },
                                ]),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    )
                    .await?;
                    log::info!("{INBOUND_FIREWALL_RULE_NAME} (created)");
                }
                Err(e) => return Err(e.into()),
                Ok(rule) => {
                    assert_eq!(rule.name.unwrap(), INBOUND_FIREWALL_RULE_NAME);
                    log::info!("{INBOUND_FIREWALL_RULE_NAME} (already existed)");
                }
            };
        };

        let step: _ = step.next();
        log::info!("Launching an instance...");
        let instance_name = format!(
            "fleeting-{}-{}",
            std::process::id(),
            // for dedup across hosts running fleeting:
            rand::thread_rng()
                .sample_iter(&Alphanumeric)
                .take(8)
                .map(char::from)
                .collect::<String>()
                .to_lowercase()
        );
        {
            let result = gcloud_sdk::google_rest_apis::compute_v1::instances_api::compute_instances_insert(
                &configuration,
                ComputePeriodInstancesPeriodInsertParams {
                    project: self.project.to_owned(),
                    zone: self.zone.to_owned(),

                    instance: Some(Instance {
                        name: Some(instance_name.clone()),
                        machine_type: Some(format!("zones/{}/machineTypes/{}", self.zone, self.machine_type)),
                        disks: Some(vec![AttachedDisk {
                            boot: Some(true),
                            auto_delete: Some(true),
                            initialize_params: Some(Box::new(AttachedDiskInitializeParams {
                                disk_size_gb: self.disk.map(|n| n.to_string()),
                                disk_type: Some(format!("zones/{}/diskTypes/pd-balanced", self.zone)), // SSD
                                source_image: Some(source_image),
                                ..Default::default()
                            })),
                            ..Default::default()
                        }]),
                        tags: Some(Box::new(Tags { items: Some(vec![INSTANCE_TAG.to_owned()]), ..Default::default() })),
                        network_interfaces: Some(vec![NetworkInterface {
                            access_configs: Some(vec![AccessConfig { ..Default::default() }]),
                            ..Default::default()
                        }]),
                        scheduling: Some(Box::new(Scheduling {
                            // Compute Engine can automatically restart VM instances if they are terminated for non-user-initiated reasons (maintenance event, hardware failure, software failure and so on)
                            // For fleeting, it makes no sense to restart, the connection will have been lost.
                            automatic_restart: Some(false),

                            // Choose what happens to your VM when it’s preempted or reaches its time limit
                            // instance_termination_action: Some("DELETE".to_owned()),

                            // termination_time / max_run_duration are not yet available in SDKs :(
                            // https://raw.githubusercontent.com/APIs-guru/openapi-directory/main/APIs/googleapis.com/compute/v1/openapi.yaml
                            // https://raw.githubusercontent.com/googleapis/googleapis/master/google/cloud/compute/v1/compute.proto
                            ..Default::default()
                        })),
                        metadata: Some(Box::new(Metadata {
                            items: Some(vec![MetadataItemsInner {
                                key: Some("startup-script".to_owned()),
                                value: Some(user_data.to_owned()),
                            }]),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await;

            if let Err(e) = result {
                // Explicitly use Debug selector, because Display (which we normally use) is useless in this SDK
                anyhow::bail!("failed to launch instance: {e:#?}");
            }
        };

        let step: _ = step.next();
        log::info!("Waiting for instance to start...");
        let public_ip = {
            let instance = loop {
                log::debug!("Retrieving instance status...");
                let instance = gcloud_sdk::google_rest_apis::compute_v1::instances_api::compute_instances_get(
                    &configuration,
                    ComputePeriodInstancesPeriodGetParams {
                        project: self.project.to_owned(),
                        zone: self.zone.to_owned(),
                        instance: instance_name.to_owned(),
                        ..Default::default()
                    },
                )
                .await?;

                // See: https://cloud.google.com/compute/docs/instances/instance-life-cycle
                match instance.status.unwrap() {
                    Status::Provisioning | Status::Staging => sleep(Duration::from_secs(1)).await,
                    Status::Running => break instance,
                    unexpected => anyhow::bail!("instance transitioned into state: {unexpected:?}"),
                }
            };

            instance
                .network_interfaces
                .expect_one("network interface")
                .access_configs
                .expect_one("access configs")
                .nat_ip
                .expect("nat ip")
                .parse()
                .expect("parsable ip")
        };

        steps::end(step);
        Ok(public_ip)
    }
}

trait OptionVecExt<T> {
    fn expect_one(self, msg: &str) -> T;
}

impl<T> OptionVecExt<T> for Option<Vec<T>> {
    fn expect_one(self, msg: &str) -> T {
        let vec = self.unwrap_or_default();
        assert_eq!(vec.len(), 1, "expected exactly one: {msg}");
        vec.into_iter().nth(0).unwrap()
    }
}
