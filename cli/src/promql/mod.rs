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
