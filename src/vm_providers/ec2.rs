use super::VmProvider;
use crate::steps;
use async_trait::async_trait;
use aws_config::{meta::region::RegionProviderChain, Region};
use aws_sdk_ec2::{
    self as ec2,
    types::{ArchitectureType, BlockDeviceMapping, EbsBlockDevice, InstanceStateName, InstanceType, ResourceType, ShutdownBehavior, Tag, TagSpecification},
};
use aws_sdk_sts::{self as sts};
use base64::prelude::*;
use clap::Args;
use std::net::Ipv4Addr;
use tokio::time::{sleep, Duration};

const SECURITY_GROUP_NAME: &str = "fleeting";

/// AWS Elastic Compute Cloud
#[derive(Args, Clone)]
#[command(
    override_usage = color_print::cstr! {r#"<bold>fleeting</bold> <bold>ec2</bold> [OPTIONS] [COMMAND]...

<bold><underline>Authentication:</underline></bold>
  - Environment variables (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY)
  - Shared config (~/.aws/config, ~/.aws/credentials)
  - Web Identity Tokens
  - ECS (IAM Roles for Tasks) & General HTTP credentials
  - EC2 IMDSv2

More info:
https://docs.rs/aws-config/1.5.5/aws_config/default_provider/credentials/struct.DefaultCredentialsChain.html

"#},)]
pub struct Ec2 {
    /// [default: $AWS[_DEFAULT]_REGION > profile > EC2 IMDSv2 > us-east-1]
    #[arg(long)]
    region: Option<String>,

    #[arg(long, default_value = "t4g.nano")]
    instance_type: InstanceType,

    /// Disk size, in GiBs.
    #[arg(long)]
    disk: Option<usize>,
}

#[async_trait]
impl VmProvider for Ec2 {
    async fn spawn(&self, user_data: &str) -> anyhow::Result<Ipv4Addr> {
        let step = steps::start();
        log::info!("Loading AWS configuration...");
        let ec2_client = {
            // TODO: use webpki_roots?
            // let https_connector = hyper_rustls::HttpsConnectorBuilder::new().with_webpki_roots().https_or_http().enable_http1().build();
            // let hyper_client = aws_smithy_runtime::client::http::hyper_014::HyperClientBuilder::new().build(https_connector);
            // let aws_config = aws_config::defaults(aws_config::BehaviorVersion::v2024_03_28()).http_client(hyper_client).load().await;

            let config = aws_config::defaults(aws_config::BehaviorVersion::v2024_03_28())
                .region(
                    RegionProviderChain::first_try(self.region.clone().map(Region::new))
                        .or_default_provider()
                        .or_else("us-east-1"),
                )
                .load()
                .await;
            log::info!("Region: {}", config.region().expect("default set"));

            log::debug!("Validating credentials...");
            let sts_client = sts::Client::new(&config);
            let caller_identity = sts_client.get_caller_identity().send().await?;
            log::info!("Identity: {}", caller_identity.arn().expect("arn"));

            ec2::Client::new(&config)
        };

        let step: _ = step.next();
        log::info!("Looking up instance type...");
        let image_id = {
            let output = ec2_client.describe_instance_types().instance_types(self.instance_type.clone()).send().await?;
            let instance_type_info = output.instance_types.expect_one("instance_type");
            let instance_type_archs = instance_type_info
                .processor_info
                .expect("processor_info")
                .supported_architectures
                .expect("supported_architectures");

            if instance_type_archs.contains(&ArchitectureType::Arm64) {
                "resolve:ssm:/aws/service/canonical/ubuntu/server/24.04/stable/current/arm64/hvm/ebs-gp3/ami-id"
            } else if instance_type_archs.contains(&ArchitectureType::X8664) {
                "resolve:ssm:/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id"
            } else {
                anyhow::bail!("unsupported instance type architectures: {instance_type_archs:?}")
            }
        };

        let step: _ = step.next();
        log::info!("Creating security group if needed...");
        let security_group_id = get_or_create_security_group(ec2_client.clone()).await?;

        let step: _ = step.next();
        log::info!("Launching an instance...");
        let instance_id = {
            // TODO: disk size. here? in global?
            let output = ec2_client
                .run_instances()
                .image_id(image_id)
                .instance_type(self.instance_type.clone())
                .user_data(BASE64_STANDARD.encode(user_data))
                .instance_initiated_shutdown_behavior(ShutdownBehavior::Terminate)
                .security_group_ids(security_group_id)
                .block_device_mappings(
                    BlockDeviceMapping::builder()
                        .device_name("/dev/sda1")
                        .ebs(
                            EbsBlockDevice::builder()
                                .delete_on_termination(true)
                                .set_volume_size(self.disk.map(|n| n.try_into().expect("valid disk size")))
                                .build(),
                        )
                        .build(),
                )
                .tag_specifications(
                    TagSpecification::builder()
                        .resource_type(ResourceType::Instance)
                        .tags(Tag::builder().key("Name").value("fleeting").build())
                        .build(),
                )
                .tag_specifications(
                    TagSpecification::builder()
                        .resource_type(ResourceType::Volume)
                        .tags(Tag::builder().key("Name").value("fleeting").build())
                        .build(),
                )
                .min_count(1)
                .max_count(1)
                .send()
                .await?;

            output.instances.expect_one("instance").instance_id.expect("instance_id")
        };
        log::info!("{instance_id}");

        let step: _ = step.next();
        log::info!("Waiting for instance to start...");
        let public_ip = {
            let instance = loop {
                log::debug!("Retrieving instance status...");
                let output = match ec2_client.describe_instances().instance_ids(&instance_id).send().await {
                    Ok(output) => output,
                    Err(e) => {
                        if e.as_service_error().and_then(|e| e.meta().code()) == Some("InvalidInstanceID.NotFound") {
                            log::debug!("Instance not found (momentarily expected due to eventual consistency)");
                            sleep(Duration::from_secs(1)).await;
                            continue;
                        } else {
                            anyhow::bail!(e)
                        }
                    }
                };

                let instance = output.reservations.expect_one("reservation").instances.expect_one("instance");
                match instance.state().expect("state").name().expect("name") {
                    InstanceStateName::Pending => sleep(Duration::from_secs(1)).await,
                    InstanceStateName::Running => break instance,
                    state => anyhow::bail!("instance transitioned into state: {state}"),
                }
            };
            instance.public_ip_address.expect("public_ip").parse().expect("valid ipv4")
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

async fn create_security_group(ec2_client: ec2::Client) -> Result<String, anyhow::Error> {
    let output = ec2_client
        .create_security_group()
        .group_name(SECURITY_GROUP_NAME)
        .description("fleeting ephemeral instances")
        .send()
        .await?;
    let group_id = output.group_id().unwrap();

    ec2_client
        .authorize_security_group_ingress()
        .group_id(group_id)
        .ip_protocol("-1")
        .cidr_ip("0.0.0.0/0")
        .send()
        .await?;

    log::info!("{group_id} (created)");
    Ok(group_id.to_string())
}

async fn get_or_create_security_group(ec2_client: ec2::Client) -> Result<String, anyhow::Error> {
    let result = ec2_client.describe_security_groups().group_names(SECURITY_GROUP_NAME).send().await;

    match result {
        Ok(output) => match output.security_groups() {
            [] => create_security_group(ec2_client).await,
            [sg] => {
                let group_id = sg.group_id().unwrap();
                log::info!("{group_id} (already existed)");
                Ok(group_id.to_string())
            }
            x => Err(anyhow::anyhow!("{} matching security groups", x.len())),
        },
        Err(e) => match e.as_service_error().and_then(|e| e.meta().code()) {
            Some("InvalidGroup.NotFound") => create_security_group(ec2_client).await,
            _ => Err(anyhow::anyhow!("error while describing security groups: {:?}", e)),
        },
    }
}
