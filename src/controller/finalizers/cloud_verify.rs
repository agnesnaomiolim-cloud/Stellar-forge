// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Fail-closed CSI volume verification for finalizer recovery.
//!
//! Kubernetes attachment absence is never sufficient evidence. AWS/GCP checks
//! use the installed provider CLI and its existing workload identity. The stock
//! distroless image has neither CLI; recovery then remains blocked. No cloud
//! credentials are created and provider command output/errors are never logged.

use std::{collections::BTreeSet, process::Stdio, sync::Arc, time::Duration};

use async_trait::async_trait;
use k8s_openapi::api::{
    core::v1::{PersistentVolume, PersistentVolumeClaim, Pod},
    storage::v1::VolumeAttachment,
};
use kube::{api::{Api, ListParams}, Client, ResourceExt};
use serde_json::Value;
use tokio::{io::AsyncReadExt, process::Command};

use crate::error::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudProvider {
    AwsEbs,
    GcpPd,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VolumeAttachmentState {
    Detached,
    /// Reserved for an authoritative, identity-verified deletion response.
    /// CLI errors and empty query results never produce this state.
    NotFound,
    Attached { instance_ids: Vec<String> },
    ApiError(String),
}

#[derive(Clone, Debug)]
pub struct VerificationResult {
    pub pvc_name: String,
    pub namespace: String,
    pub volume_id: String,
    pub provider: CloudProvider,
    pub state: VolumeAttachmentState,
    pub safe_to_release: bool,
}

#[derive(Clone, Debug)]
pub struct VolumeIdentity {
    pub provider: CloudProvider,
    pub volume_id: String,
    pub region: String,
    pub zone: String,
    pub project: String,
    pub regional: bool,
}

#[async_trait]
pub trait CloudChecker: Send + Sync {
    async fn check(&self, volume: &VolumeIdentity) -> VolumeAttachmentState;
}

struct ProviderCliChecker;

#[async_trait]
impl CloudChecker for ProviderCliChecker {
    async fn check(&self, volume: &VolumeIdentity) -> VolumeAttachmentState {
        let response = match volume.provider {
            CloudProvider::AwsEbs => {
                let suffix = if volume.region.starts_with("cn-") {
                    "amazonaws.com.cn"
                } else {
                    "amazonaws.com"
                };
                let endpoint = format!("https://ec2.{}.{suffix}", volume.region);
                provider_json("aws", &[
                    "ec2", "describe-volumes", "--volume-ids", &volume.volume_id,
                    "--region", &volume.region, "--endpoint-url", &endpoint,
                    "--output", "json", "--no-cli-pager", "--query",
                    "Volumes[].{VolumeId:VolumeId,AvailabilityZone:AvailabilityZone,State:State,Attachments:Attachments[].{InstanceId:InstanceId,State:State}}",
                ]).await
            }
            CloudProvider::GcpPd => {
                let scope = if volume.regional { "--region" } else { "--zone" };
                let location = if volume.regional { &volume.region } else { &volume.zone };
                provider_json("gcloud", &[
                    "compute", "disks", "describe", &volume.volume_id,
                    "--project", &volume.project, scope, location,
                    "--format=json", "--quiet",
                ]).await
            }
        };
        match response {
            Ok(value) => match volume.provider {
                CloudProvider::AwsEbs => parse_aws(volume, &value),
                CloudProvider::GcpPd => parse_gcp(volume, &value),
            },
            Err(message) => VolumeAttachmentState::ApiError(message),
        }
    }
}

/// No shell, no interactive login, bounded output/time, kill on timeout/drop.
async fn provider_json(program: &str, args: &[&str]) -> std::result::Result<Value, String> {
    const MAX_OUTPUT: u64 = 64 * 1024;
    let operation = async {
        let mut child = Command::new(program)
            .args(args)
            .env("AWS_PAGER", "")
            .env("AWS_CLI_AUTO_PROMPT", "off")
            .env("CLOUDSDK_CORE_DISABLE_PROMPTS", "1")
            .env("CLOUDSDK_API_ENDPOINT_OVERRIDES_COMPUTE", "https://compute.googleapis.com/compute/v1/")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "provider CLI unavailable".to_string())?;
        let mut bytes = Vec::new();
        child.stdout.take().ok_or("provider stdout unavailable")?
            .take(MAX_OUTPUT + 1).read_to_end(&mut bytes).await
            .map_err(|_| "provider response read failed".to_string())?;
        if bytes.len() as u64 > MAX_OUTPUT {
            return Err("provider response exceeds limit".to_string());
        }
        let status = child.wait().await.map_err(|_| "provider process failed".to_string())?;
        if !status.success() {
            // Includes access denied, missing credentials and not-found errors.
            return Err("provider lookup failed; verify CLI, identity and read permission".to_string());
        }
        serde_json::from_slice(&bytes).map_err(|_| "invalid provider JSON response".to_string())
    };
    tokio::time::timeout(Duration::from_secs(20), operation).await
        .map_err(|_| "provider lookup timed out".to_string())?
}

fn invalid(message: &str) -> VolumeAttachmentState {
    VolumeAttachmentState::ApiError(message.to_string())
}

fn parse_aws(volume: &VolumeIdentity, response: &Value) -> VolumeAttachmentState {
    let Some(volumes) = response.as_array() else { return invalid("invalid EBS response"); };
    if volumes.len() != 1 {
        return invalid("EBS lookup did not return exactly one volume");
    }
    let value = &volumes[0];
    if value["VolumeId"].as_str() != Some(volume.volume_id.as_str())
        || value["AvailabilityZone"].as_str() != Some(volume.zone.as_str()) {
        return invalid("EBS volume identity mismatch");
    }
    let Some(attachments) = value["Attachments"].as_array() else {
        return invalid("EBS attachment state missing");
    };
    if !attachments.is_empty() {
        return VolumeAttachmentState::Attached {
            instance_ids: attachments.iter().filter_map(|a| a["InstanceId"].as_str().map(str::to_string)).collect(),
        };
    }
    if value["State"].as_str() == Some("available") {
        VolumeAttachmentState::Detached
    } else {
        invalid("EBS volume is not available")
    }
}

fn parse_gcp(volume: &VolumeIdentity, response: &Value) -> VolumeAttachmentState {
    let scope = if volume.regional { "regions" } else { "zones" };
    let location = if volume.regional { &volume.region } else { &volume.zone };
    let expected = format!("https://www.googleapis.com/compute/v1/projects/{}/{scope}/{location}/disks/{}", volume.project, volume.volume_id);
    if response["name"].as_str() != Some(volume.volume_id.as_str())
        || response["selfLink"].as_str() != Some(expected.as_str()) {
        return invalid("GCP disk identity mismatch");
    }
    if let Some(users) = response.get("users") {
        let Some(users) = users.as_array() else { return invalid("invalid GCP disk users"); };
        if !users.is_empty() {
            return VolumeAttachmentState::Attached {
                instance_ids: users.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
            };
        }
    }
    if response["status"].as_str() == Some("READY") {
        VolumeAttachmentState::Detached
    } else {
        invalid("GCP disk is not READY")
    }
}

fn component(value: &str) -> bool {
    !value.is_empty() && value.len() <= 253 && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

fn volume_identity(pv: &PersistentVolume) -> Result<VolumeIdentity> {
    let spec = pv.spec.as_ref().ok_or_else(|| Error::FinalizerError("PV spec missing".into()))?;
    let csi = spec.csi.as_ref().ok_or_else(|| Error::FinalizerError("recovery supports only identified EBS/GCP CSI volumes".into()))?;
    let mut identity = VolumeIdentity {
        provider: CloudProvider::AwsEbs, volume_id: csi.volume_handle.clone(),
        region: String::new(), zone: String::new(), project: String::new(), regional: false,
    };
    match csi.driver.as_str() {
        "ebs.csi.aws.com" => {
            if !identity.volume_id.starts_with("vol-") || !identity.volume_id[4..].bytes().all(|b| b.is_ascii_hexdigit()) || identity.volume_id.len() <= 4 {
                return Err(Error::FinalizerError("invalid EBS CSI volume handle".into()));
            }
            let mut zones = BTreeSet::new();
            if let Some(labels) = &pv.metadata.labels {
                for key in ["topology.kubernetes.io/zone", "failure-domain.beta.kubernetes.io/zone"] {
                    if let Some(zone) = labels.get(key) { zones.insert(zone.clone()); }
                }
            }
            if let Some(terms) = spec.node_affinity.as_ref().and_then(|a| a.required.as_ref()).map(|s| &s.node_selector_terms) {
                for term in terms {
                    for expression in term.match_expressions.iter().flatten() {
                        if ["topology.ebs.csi.aws.com/zone", "topology.kubernetes.io/zone", "failure-domain.beta.kubernetes.io/zone"].contains(&expression.key.as_str()) && expression.operator == "In" {
                            zones.extend(expression.values.iter().flatten().cloned());
                        }
                    }
                }
            }
            if zones.len() != 1 { return Err(Error::FinalizerError("EBS volume requires one unambiguous topology zone".into())); }
            identity.zone = zones.into_iter().next().unwrap();
            // Local/Wavelength/ISO zones need dedicated endpoint/region support;
            // guessing a region could query the wrong physical volume.
            let Some(region) = identity.zone.strip_suffix(|c: char| c.is_ascii_lowercase()) else {
                return Err(Error::FinalizerError("unsupported EBS zone".into()));
            };
            let parts: Vec<_> = region.split('-').collect();
            if parts.len() < 3 || parts[..parts.len()-1].iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_lowercase()))
                || parts.last().unwrap().is_empty() || !parts.last().unwrap().bytes().all(|b| b.is_ascii_digit()) || region.contains("iso") {
                return Err(Error::FinalizerError("unsupported EBS region topology".into()));
            }
            identity.region = region.to_string();
        }
        "pd.csi.storage.gke.io" => {
            let parts: Vec<_> = csi.volume_handle.split('/').collect();
            if parts.len() != 6 || parts[0] != "projects" || !["zones", "regions"].contains(&parts[2])
                || parts[4] != "disks" || ![parts[1], parts[3], parts[5]].iter().all(|v| component(v)) {
                return Err(Error::FinalizerError("invalid GCP CSI disk identity".into()));
            }
            identity.provider = CloudProvider::GcpPd;
            identity.project = parts[1].to_string();
            identity.regional = parts[2] == "regions";
            identity.volume_id = parts[5].to_string();
            if identity.regional { identity.region = parts[3].to_string(); }
            else { identity.zone = parts[3].to_string(); }
        }
        _ => return Err(Error::FinalizerError("unsupported CSI provider; recovery blocked".into())),
    }
    Ok(identity)
}

pub struct CloudVerifier {
    client: Client,
    checker: Arc<dyn CloudChecker>,
}

impl CloudVerifier {
    pub fn new(client: Client) -> Self {
        Self { client, checker: Arc::new(ProviderCliChecker) }
    }

    pub fn with_checker(client: Client, checker: Arc<dyn CloudChecker>) -> Self {
        Self { client, checker }
    }

    pub async fn verify_volume_detached(&self, pvc: &PersistentVolumeClaim) -> Result<VerificationResult> {
        // Bound the entire operation, including Kubernetes pagination.
        tokio::time::timeout(Duration::from_secs(60), self.verify(pvc)).await
            .map_err(|_| Error::FinalizerError("volume verification timed out".into()))?
    }

    async fn verify(&self, pvc: &PersistentVolumeClaim) -> Result<VerificationResult> {
        let namespace = pvc.namespace().ok_or_else(|| Error::FinalizerError("PVC namespace missing".into()))?;
        let name = pvc.name_any();
        let uid = pvc.metadata.uid.as_deref().filter(|u| !u.is_empty()).ok_or_else(|| Error::FinalizerError("PVC UID missing".into()))?;
        let pv_name = pvc.spec.as_ref().and_then(|s| s.volume_name.as_deref()).filter(|s| !s.is_empty()).ok_or_else(|| Error::FinalizerError("PVC volume binding missing".into()))?;
        let pv_api: Api<PersistentVolume> = Api::all(self.client.clone());
        // A Kubernetes 404 does not establish provider-side deletion.
        let pv = pv_api.get(pv_name).await.map_err(Error::KubeError)?;
        let claim = pv.spec.as_ref().and_then(|s| s.claim_ref.as_ref()).ok_or_else(|| Error::FinalizerError("PV claim binding missing".into()))?;
        if claim.uid.as_deref() != Some(uid) || claim.name.as_deref() != Some(name.as_str()) || claim.namespace.as_deref() != Some(namespace.as_str()) {
            return Err(Error::FinalizerError("PV claim identity mismatch".into()));
        }
        let identity = volume_identity(&pv)?;
        let first = self.kubernetes_state(&namespace, &name, pv_name).await?;
        let mut state = match first {
            Some(state) => state,
            None => self.checker.check(&identity).await,
        };
        if matches!(state, VolumeAttachmentState::Detached | VolumeAttachmentState::NotFound) {
            // Recheck after the cloud round trip; attachment intent and live Pod
            // references override a cloud response that says detached.
            if let Some(current) = self.kubernetes_state(&namespace, &name, pv_name).await? { state = current; }
            let current = pv_api.get(pv_name).await.map_err(Error::KubeError)?;
            if current.metadata.uid != pv.metadata.uid || current.spec != pv.spec {
                return Err(Error::FinalizerError("PV changed during verification".into()));
            }
            let claims: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), &namespace);
            let current_claim = claims.get(&name).await.map_err(Error::KubeError)?;
            if current_claim.metadata.uid.as_deref() != Some(uid)
                || current_claim.spec.as_ref().and_then(|s| s.volume_name.as_deref()) != Some(pv_name) {
                return Err(Error::FinalizerError("PVC changed during verification".into()));
            }
        }
        let safe_to_release = matches!(state, VolumeAttachmentState::Detached | VolumeAttachmentState::NotFound);
        Ok(VerificationResult { pvc_name: name, namespace, volume_id: identity.volume_id, provider: identity.provider, state, safe_to_release })
    }

    async fn kubernetes_state(&self, namespace: &str, pvc_name: &str, pv_name: &str) -> Result<Option<VolumeAttachmentState>> {
        let attachments: Api<VolumeAttachment> = Api::all(self.client.clone());
        let mut params = ListParams::default().limit(200);
        loop {
            let page = attachments.list(&params).await.map_err(Error::KubeError)?;
            for attachment in page.items {
                // Even attached=false is attachment intent, including pending or
                // failed attachment/detachment. Wait until the object is removed.
                if attachment.spec.source.persistent_volume_name.as_deref() == Some(pv_name) {
                    return Ok(Some(VolumeAttachmentState::Attached { instance_ids: vec![attachment.spec.node_name] }));
                }
            }
            match page.metadata.continue_.filter(|s| !s.is_empty()) {
                Some(token) => params = ListParams::default().limit(200).continue_token(&token),
                None => break,
            }
        }
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let mut params = ListParams::default().limit(200);
        loop {
            let page = pods.list(&params).await.map_err(Error::KubeError)?;
            for pod in page.items {
                let terminal = matches!(pod.status.as_ref().and_then(|s| s.phase.as_deref()), Some("Succeeded" | "Failed"));
                if !terminal && pod.spec.as_ref().is_some_and(|spec| spec.volumes.iter().flatten().any(|v| v.persistent_volume_claim.as_ref().is_some_and(|claim| claim.claim_name == pvc_name))) {
                    return Ok(Some(VolumeAttachmentState::Attached { instance_ids: vec![pod.name_any()] }));
                }
            }
            match page.metadata.continue_.filter(|s| !s.is_empty()) {
                Some(token) => params = ListParams::default().limit(200).continue_token(&token),
                None => break,
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
pub struct MockChecker {
    pub state: VolumeAttachmentState,
}

#[cfg(test)]
#[async_trait]
impl CloudChecker for MockChecker {
    async fn check(&self, _volume: &VolumeIdentity) -> VolumeAttachmentState { self.state.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{CSIPersistentVolumeSource, PersistentVolumeSpec};
    use serde_json::json;

    fn ebs() -> VolumeIdentity {
        VolumeIdentity {
            provider: CloudProvider::AwsEbs, volume_id: "vol-0123456789abcdef0".into(),
            region: "us-east-1".into(), zone: "us-east-1a".into(),
            project: String::new(), regional: false,
        }
    }

    fn gcp(regional: bool) -> VolumeIdentity {
        VolumeIdentity {
            provider: CloudProvider::GcpPd, volume_id: "validator-disk".into(),
            region: "us-central1".into(), zone: "us-central1-a".into(),
            project: "validator-project".into(), regional,
        }
    }

    fn pv(driver: &str, handle: &str) -> PersistentVolume {
        PersistentVolume {
            spec: Some(PersistentVolumeSpec {
                csi: Some(CSIPersistentVolumeSource {
                    driver: driver.into(), volume_handle: handle.into(), ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn aws_requires_available_exact_volume_and_explicit_empty_attachments() {
        let volume = ebs();
        let good = json!([{
            "VolumeId": volume.volume_id, "AvailabilityZone": volume.zone,
            "State": "available", "Attachments": []
        }]);
        assert_eq!(parse_aws(&volume, &good), VolumeAttachmentState::Detached);
        for bad in [json!([]), json!({}), json!([{}])] {
            assert!(matches!(parse_aws(&volume, &bad), VolumeAttachmentState::ApiError(_)));
        }
        for (field, wrong) in [
            ("VolumeId", json!("vol-incorrect")),
            ("AvailabilityZone", json!("us-west-2a")),
            ("Attachments", Value::Null),
            ("State", json!("in-use")),
            ("State", json!("deleting")),
        ] {
            let mut bad = good.clone();
            bad[0][field] = wrong;
            assert!(matches!(parse_aws(&volume, &bad), VolumeAttachmentState::ApiError(_)));
        }
    }

    #[test]
    fn aws_pending_and_detaching_attachments_block_recovery() {
        let volume = ebs();
        for status in ["attaching", "attached", "detaching", "detached"] {
            let response = json!([{
                "VolumeId": volume.volume_id, "AvailabilityZone": volume.zone,
                "State": "available", "Attachments": [{"InstanceId":"i-1", "State":status}]
            }]);
            assert!(matches!(parse_aws(&volume, &response), VolumeAttachmentState::Attached { .. }));
        }
    }

    #[test]
    fn gcp_zonal_and_regional_disks_require_matching_identity() {
        for regional in [false, true] {
            let volume = gcp(regional);
            let scope = if regional { "regions/us-central1" } else { "zones/us-central1-a" };
            let mut response = json!({
                "name":"validator-disk", "status":"READY",
                "selfLink":format!("https://www.googleapis.com/compute/v1/projects/validator-project/{scope}/disks/validator-disk")
            });
            // Compute omits empty repeated fields; omitted users is valid only
            // after the full disk identity and READY state have been verified.
            assert_eq!(parse_gcp(&volume, &response), VolumeAttachmentState::Detached);
            response["users"] = json!(["projects/p/zones/z/instances/i"]);
            assert!(matches!(parse_gcp(&volume, &response), VolumeAttachmentState::Attached { .. }));
            response["users"] = Value::Null;
            assert!(matches!(parse_gcp(&volume, &response), VolumeAttachmentState::ApiError(_)));
            response["users"] = json!([]);
            response["status"] = json!("DELETING");
            assert!(matches!(parse_gcp(&volume, &response), VolumeAttachmentState::ApiError(_)));
            response["status"] = json!("READY");
            response["selfLink"] = json!("https://www.googleapis.com/compute/v1/projects/wrong/zones/us-central1-a/disks/validator-disk");
            assert!(matches!(parse_gcp(&volume, &response), VolumeAttachmentState::ApiError(_)));
        }
    }

    #[test]
    fn unsupported_missing_and_malformed_identities_fail_closed() {
        assert!(volume_identity(&pv("unknown.csi", "disk")).is_err());
        assert!(volume_identity(&pv("pd.csi.storage.gke.io", "validator-disk")).is_err());
        assert!(volume_identity(&pv("pd.csi.storage.gke.io", "projects/--config/zones/a/disks/d")).is_err());
        assert!(volume_identity(&pv("pd.csi.storage.gke.io", "projects/p/zones/a/disks/d/extra")).is_err());
        assert!(volume_identity(&pv("ebs.csi.aws.com", "vol-0123456789abcdef0")).is_err());
        assert!(volume_identity(&PersistentVolume::default()).is_err());
    }

    #[test]
    fn ebs_zone_is_required_and_unsupported_zones_are_not_guessed() {
        let mut volume = pv("ebs.csi.aws.com", "vol-0123456789abcdef0");
        for (zone, valid) in [("us-east-1a", true), ("us-gov-west-1a", true), ("us-west-2-lax-1a", false), ("us-iso-east-1a", false), ("us-east-a", false)] {
            volume.metadata.labels = Some(std::collections::BTreeMap::from([
                ("topology.kubernetes.io/zone".into(), zone.into())
            ]));
            assert_eq!(volume_identity(&volume).is_ok(), valid);
        }
    }

    #[test]
    fn gcp_identity_preserves_explicit_project_and_scope() {
        let volume = volume_identity(&pv("pd.csi.storage.gke.io", "projects/my-project/regions/us-central1/disks/my-disk")).unwrap();
        assert_eq!(volume.provider, CloudProvider::GcpPd);
        assert_eq!(volume.project, "my-project");
        assert_eq!(volume.region, "us-central1");
        assert_eq!(volume.volume_id, "my-disk");
        assert!(volume.regional);
    }

    #[tokio::test]
    async fn mock_checks_preserve_errors_without_invoking_provider_commands() {
        for state in [
            VolumeAttachmentState::Detached,
            VolumeAttachmentState::Attached { instance_ids: vec!["instance".into()] },
            VolumeAttachmentState::ApiError("denied".into()),
        ] {
            let checker = MockChecker { state: state.clone() };
            assert_eq!(checker.check(&ebs()).await, state);
        }
    }

    // A tiny local Kubernetes HTTP fixture; provider subprocesses are never
    // invoked by these tests. Each response closes its connection explicitly.
    async fn kubernetes_fixture(responses: Vec<(u16, Value)>) -> (Client, tokio::task::JoinHandle<()>) {
        use tokio::{io::AsyncWriteExt, net::TcpListener};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).await.unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 8192);
                    if request.ends_with(b"\r\n\r\n") { break; }
                }
                let body = body.to_string();
                let response = format!("HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let config = kube::Config::new(format!("http://{address}").parse().unwrap());
        (Client::try_from(config).unwrap(), server)
    }

    fn bound_claim() -> PersistentVolumeClaim {
        serde_json::from_value(json!({
            "apiVersion":"v1", "kind":"PersistentVolumeClaim",
            "metadata":{"name":"data", "namespace":"stellar", "uid":"claim-1"},
            "spec":{"volumeName":"pv-1"}
        })).unwrap()
    }

    fn bound_volume() -> Value {
        json!({
            "apiVersion":"v1", "kind":"PersistentVolume",
            "metadata":{"name":"pv-1", "uid":"volume-1"},
            "spec":{
                "claimRef":{"name":"data", "namespace":"stellar", "uid":"claim-1"},
                "csi":{"driver":"pd.csi.storage.gke.io", "volumeHandle":"projects/my-project/zones/us-central1-a/disks/disk-1"}
            }
        })
    }

    fn empty_list(kind: &str) -> Value {
        json!({"apiVersion":"v1", "kind":kind, "metadata":{}, "items":[]})
    }

    struct UnexpectedCloudCheck;

    #[async_trait]
    impl CloudChecker for UnexpectedCloudCheck {
        async fn check(&self, _volume: &VolumeIdentity) -> VolumeAttachmentState {
            panic!("cloud check must not run without Kubernetes safety evidence")
        }
    }

    #[tokio::test]
    async fn kubernetes_not_found_or_rbac_denial_never_proves_detachment() {
        for (code, reason) in [(404, "NotFound"), (403, "Forbidden")] {
            let status = json!({"kind":"Status", "apiVersion":"v1", "status":"Failure", "reason":reason, "message":reason, "code":code});
            let (client, server) = kubernetes_fixture(vec![(code, status)]).await;
            let verifier = CloudVerifier::with_checker(client, Arc::new(UnexpectedCloudCheck));
            assert!(verifier.verify_volume_detached(&bound_claim()).await.is_err());
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn pending_csi_attachment_blocks_even_with_attached_false() {
        let attachments = json!({
            "apiVersion":"storage.k8s.io/v1", "kind":"VolumeAttachmentList", "metadata":{},
            "items":[{
                "metadata":{"name":"pending-attachment"},
                "spec":{"attacher":"pd.csi.storage.gke.io", "nodeName":"node-1", "source":{"persistentVolumeName":"pv-1"}},
                "status":{"attached":false}
            }]
        });
        let (client, server) = kubernetes_fixture(vec![(200, bound_volume()), (200, attachments)]).await;
        let verifier = CloudVerifier::with_checker(client, Arc::new(UnexpectedCloudCheck));
        let result = verifier.verify_volume_detached(&bound_claim()).await.unwrap();
        assert!(!result.safe_to_release);
        assert!(matches!(result.state, VolumeAttachmentState::Attached { .. }));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn confirmed_detachment_requires_rechecking_kubernetes_identity() {
        let (client, server) = kubernetes_fixture(vec![
            (200, bound_volume()),
            (200, empty_list("VolumeAttachmentList")), (200, empty_list("PodList")),
            (200, empty_list("VolumeAttachmentList")), (200, empty_list("PodList")),
            (200, bound_volume()), (200, serde_json::to_value(bound_claim()).unwrap()),
        ]).await;
        let verifier = CloudVerifier::with_checker(client, Arc::new(MockChecker { state: VolumeAttachmentState::Detached }));
        let result = verifier.verify_volume_detached(&bound_claim()).await.unwrap();
        assert!(result.safe_to_release);
        server.await.unwrap();
    }
}
