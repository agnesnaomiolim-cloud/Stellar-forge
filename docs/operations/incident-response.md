# Incident Response

Use this overview to stabilize an operational incident, coordinate the response, and choose the runbook that matches the failure. Record the time each decision is made and keep changes limited to actions that restore or protect service.

## 1. Assess and Stabilize

Confirm the affected service, scope, start time, and current impact. Check health first, then stop automated or manual actions that could make the outage or data loss worse. Preserve evidence such as pod status, events, and recent logs before changing state.

## 2. Assign the Response

Notify the on-call incident commander and the owners of the affected service or platform. If regional failover or destructive recovery is considered, require incident-commander approval and keep the incident channel, affected users, and monitoring teams informed as the situation changes.

## 3. Establish Where to Look

Start with the Kubernetes and application logs, pod and node events, service health checks, metrics, and the latest alerts. Compare the observed symptom with the relevant runbook below before taking recovery action. See [Disaster Recovery & Quorum Loss Runbook](./disaster-recovery.md) for pod, PVC, quorum-loss, and post-incident checks.

## 4. Follow a Scenario Runbook

- For backup and restore verification, use [Disaster Recovery & Backup Verification Automation](./backup-verification.md).
- For captive-core state corruption, use [Captive Core State Corruption: Rebuild Guide](./captive-core-rebuild.md).
- For validator or service upgrades, use [Blue-Green Deployment for Stellar Core Validators](./blue-green-upgrade.md) or [Stellar Core Blue/Green Deployment Runbook](./core-blue-green-runbook.md).
- For capacity or storage growth, use [Capacity Planning & Storage Scaling Calculator Guide](./capacity-planning.md).
- For site or region loss, use [Disaster Recovery: Rebuilding Quorum After Complete Datacenter Loss](./dr-datacenter-loss.md) or [Disaster Recovery: Failover & Snapshot Restoration Runbook](./dr-failover.md).

## 5. Resolve and Record

When service is stable, verify that health checks, logs, consensus, or replication are back to normal according to the selected runbook. Document the timeline, impact, actions taken, remaining risks, backup or snapshot state, and follow-up owners. Hand off unresolved work to the responsible team and complete the post-incident review.