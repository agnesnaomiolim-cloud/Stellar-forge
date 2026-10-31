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
//! PromQL query aggregation and analytics for multi-cluster
/// performance drift detection.
///
/// This module provides an asynchronous PromQL client capable of
/// querying multiple Prometheus endpoints in parallel, statistical
/// variance analysis to detect outliers, and alert generation for
/// degrading clusters.

pub mod aggregator;
pub mod analytics;

pub use aggregator::{
    AggregationReport, ClusterMetricResult, ClusterPrometheusEndpoint,
    DegradedClusterEvent, PromQuery, PromQl Aggregator, PromQueryError,
    PrometheusAlertMetric, PrometheusClient,
};
pub use analytics:{
    drift_detection, mean, median, percentile, population_standard_deviation,
    sample_standard_deviation, variance, DriftAnalysis, DriftConfig, DriftOutlier,
    DriftSeverity, StatisticalSummary,
};