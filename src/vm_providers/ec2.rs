use super::VmProvider;
use async_trait::async_trait;
use aws_config::{meta::region::RegionProviderChain, Region};
use aws_sdk_ec2::{
    types::{ArchitectureType, InstanceStateName, InstanceType, ResourceType, ShutdownBehavior, Tag, TagSpecification},
    Client,
};
use base64::prelude::*;
use clap::Args;
use std::net::Ipv4Addr;
use tokio::time::{sleep, Duration};

#[derive(Args, Clone)]
pub struct Ec2 {
    /// [default: $AWS[_DEFAULT]_REGION > profile > EC2 IMDSv2 > us-east-1]
    #[arg(long)]
    region: Option<String>,

    #[arg(long, default_value = "t4g.nano")]
    instance_type: InstanceType,
}

#[async_trait]
impl VmProvider for Ec2 {
    async fn spawn(&self, user_data: &str) -> anyhow::Result<Ipv4Addr> {
        // use webpki_roots?
        // let https_connector = hyper_rustls::HttpsConnectorBuilder::new().with_webpki_roots().https_or_http().enable_http1().build();
        // let hyper_client = aws_smithy_runtime::client::http::hyper_014::HyperClientBuilder::new().build(https_connector);
        // let aws_config = aws_config::defaults(aws_config::BehaviorVersion::v2024_03_28()).http_client(hyper_client).load().await;

        let config = aws_config::defaults(aws_config::BehaviorVersion::v2024_03_28())
            .region(RegionProviderChain::first_try(self.region.clone().map(Region::new)).or_default_provider().or_else("us-east-1"))
            .load()
            .await;
        let client = Client::new(&config);

        println!("Looking up instance type...");
        let output = client.describe_instance_types().instance_types(self.instance_type.clone()).send().await?;
        let instance_type_info = output.instance_types.expect_one("instance_type");
        let instance_type_archs = instance_type_info.processor_info.expect("processor_info").supported_architectures.expect("supported_architectures");

        let image_id = if instance_type_archs.contains(&ArchitectureType::Arm64) {
            "resolve:ssm:/aws/service/canonical/ubuntu/server/24.04/stable/current/arm64/hvm/ebs-gp3/ami-id"
        } else if instance_type_archs.contains(&ArchitectureType::X8664) {
            "resolve:ssm:/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id"
        } else {
            anyhow::bail!("unsupported instance type architectures: {instance_type_archs:?}")
        };

        // aws ec2 create-security-group \
        //     --group-name fleeting \
        //     --description 'fleeting ephemeral instances'

        // aws ec2 authorize-security-group-ingress \
        //     --group-id sg-0d9613dfa3104679c \
        //     --protocol all \
        //     --cidr 0.0.0.0/0

        println!("Launching an instance...");
        // TODO: disk size. here? in global?
        let output = client
            .run_instances()
            .image_id(image_id)
            .instance_type(self.instance_type.clone())
            .user_data(BASE64_STANDARD.encode(user_data))
            .instance_initiated_shutdown_behavior(ShutdownBehavior::Terminate)
            .security_group_ids("sg-0d9613dfa3104679c")
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

        let instance_id = output.instances.expect_one("instance").instance_id.expect("instance_id");
        println!("Instance launched ({instance_id}), waiting for it to start");

        let instance = loop {
            println!("Retrieving instance status");
            let output = match client.describe_instances().instance_ids(&instance_id).send().await {
                Ok(output) => output,
                Err(e) => {
                    if e.as_service_error().and_then(|e| e.meta().code()) == Some("InvalidInstanceID.NotFound") {
                        println!("Instance not found (momentarily expected due to eventual consistency)");
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

        let public_ip: Ipv4Addr = instance.public_ip_address.expect("public_ip").parse().expect("valid ipv4");
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
