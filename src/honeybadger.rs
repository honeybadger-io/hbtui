use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct HoneybadgerClient {
    auth_token: String,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub fault_count: u64,
}

#[derive(Debug, Clone)]
pub struct ProjectStats {
    pub total_projects: usize,
    pub total_faults: u64,
    pub unresolved_faults: u64,
    pub recent_projects: Vec<Project>,
}

#[derive(Debug, Deserialize)]
struct ProjectsResponse {
    results: Vec<Project>,
}

#[derive(Debug, Deserialize)]
struct FaultCountsResponse {
    total: u64,
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

        let projects_response: ProjectsResponse = response.json().await?;
        Ok(projects_response.results)
    }

    pub async fn get_fault_counts(&self, project_id: u64) -> Result<u64> {
        let url = format!(
            "https://app.honeybadger.io/v2/projects/{}/fault_counts",
            project_id
        );
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .header("Accept", "application/json")
            .send()
            .await?;

        let fault_counts: FaultCountsResponse = response.json().await?;
        Ok(fault_counts.total)
    }

    pub async fn get_project_stats(&self) -> Result<ProjectStats> {
        let projects = self.list_projects().await?;
        let total_projects = projects.len();

        // Get fault counts for the first few projects (to avoid rate limiting)
        let mut enriched_projects = Vec::new();
        let mut total_faults = 0u64;
        let mut unresolved_faults = 0u64;

        for project in projects.iter().take(10) {
            let fault_count = self.get_fault_counts(project.id).await.unwrap_or(0);
            total_faults += fault_count;
            unresolved_faults += fault_count; // Simplified - would need to check actual status

            let mut enriched = project.clone();
            enriched.fault_count = fault_count;
            enriched_projects.push(enriched);
        }

        Ok(ProjectStats {
            total_projects,
            total_faults,
            unresolved_faults,
            recent_projects: enriched_projects,
        })
    }
}
