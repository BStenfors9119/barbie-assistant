use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TravelRequest {
    pub id: String,
    pub employee_id: String,
    pub trip_number: String,
    pub status: TravelRequestStatus,
    pub start_date: String,
    pub end_date: String,
    pub destination: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TravelRequestStatus {
    Draft,
    Submitted,
    Approved,
    Rejected,
    Settled,
}
