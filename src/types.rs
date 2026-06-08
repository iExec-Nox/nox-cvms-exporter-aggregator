use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CvmInstance {
    pub instance_id: String,
    pub url: String,
    pub machine_id: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CvmSummary {
    pub app_id: String,
    pub name: String,
    pub instances: Vec<CvmInstance>,
}
