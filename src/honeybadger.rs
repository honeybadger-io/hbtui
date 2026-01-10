use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

use crate::dashboard::InsightsResponse;

#[derive(Debug, Clone)]
pub struct HoneybadgerClient {
    auth_token: String,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: u64,
    pub name: String,
    pub fault_count: u64,
    pub unresolved_fault_count: u64,
}

#[derive(Debug, Clone)]
pub struct ProjectStats {
    pub total_projects: usize,
    pub total_faults: u64,
    pub unresolved_faults: u64,
    pub recent_projects: Vec<Project>,
}

impl HoneybadgerClient {
    pub fn new(auth_token: String) -> Self {
        Self {
            auth_token,
            client: reqwest::Client::new(),
        }
    }

    pub async fn list_projects(&self) -> Result<Vec<Project>> {
        let url = "https://app.honeybadger.io/v2/projects";
        let response = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .header("Accept", "application/json")
            .send()
            .await?;

        let projects: Vec<Project> = response.json().await?;
        Ok(projects)
    }

    pub async fn get_project_stats(&self) -> Result<ProjectStats> {
        let projects = self.list_projects().await?;
        let total_projects = projects.len();

        let mut total_faults = 0u64;
        let mut unresolved_faults = 0u64;

        for project in &projects {
            total_faults += project.fault_count;
            unresolved_faults += project.unresolved_fault_count;
        }

        Ok(ProjectStats {
            total_projects,
            total_faults,
            unresolved_faults,
            recent_projects: projects,
        })
    }

    /// Execute an Insights query for a specific project
    pub async fn query_insights(
        &self,
        project_id: u64,
        query: &str,
    ) -> Result<InsightsResponse> {
        let url = format!(
            "https://app.honeybadger.io/v2/projects/{}/insights/queries",
            project_id
        );

        let body = serde_json::json!({
            "query": query,
        });

        // Insights API uses Basic auth (token as username, empty password)
        let auth = STANDARD.encode(format!("{}:", self.auth_token));

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Basic {}", auth))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Insights API error {}: {}", status, text));
        }

        let insights_response: InsightsResponse = response.json().await?;

        // Check for inline error in response
        if let Some(error) = &insights_response.error {
            return Err(anyhow::anyhow!("{}", error.message));
        }

        Ok(insights_response)
    }
}
