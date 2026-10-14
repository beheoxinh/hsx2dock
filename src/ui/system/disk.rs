//! Disk-usage tiles for the Engine page.

use super::*;
use crate::docker::models::DfReport;
use crate::ui::common::format_bytes;

impl SystemView {
    /// Render disk usage reported by `docker system df`.
    pub fn set_df(&self, report: &DfReport) {
        self.disk_row.set_subtitle(&format!(
            "Images {} ({}) · Containers {} ({}) · Volumes {} ({})",
            report.images_count,
            format_bytes(report.images_size),
            report.containers_count,
            format_bytes(report.containers_size),
            report.volumes_count,
            format_bytes(report.volumes_size),
        ));
        self.metric_volumes
            .set_label(&format!("{}", report.volumes_count));
        let total = report.images_size + report.containers_size + report.volumes_size;
        self.metric_disk.set_label(&format_bytes(total));
    }
}
