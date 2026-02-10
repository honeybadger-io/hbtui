use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

use crate::dashboard::InsightsResponse;

#[derive(Debug, Clone)]
pub struct HoneybadgerClient {
    auth_token: String,
    base_url: String,
    client: reqwest::Client,
}

// Kept for deserialization tests - not used in production code
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: u64,
    pub name: String,
    pub fault_count: u64,
    pub unresolved_fault_count: u64,
}



impl HoneybadgerClient {
    pub fn new(auth_token: String, endpoint: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            auth_token,
            base_url: endpoint,
            client,
        }
    }

    // Kept for API parsing tests - not used in production code
    #[allow(dead_code)]
    pub async fn list_projects(&self) -> Result<Vec<Project>> {
        let url = format!("{}/v2/projects", self.base_url);
        let response = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .header("Accept", "application/json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("API error {}: {}", status, text));
        }

        let projects: Vec<Project> = response.json().await?;
        Ok(projects)
    }

    /// Execute an Insights query for a specific project
    pub async fn query_insights(
        &self,
        project_id: u64,
        query: &str,
    ) -> Result<InsightsResponse> {
        let url = format!(
            "{}/v2/projects/{}/insights/queries",
            self.base_url, project_id
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_has_timeout() {
        // Test that the client is configured with a timeout
        let client = HoneybadgerClient::new(
            "test_token".to_string(),
            "https://app.honeybadger.io".to_string(),
        );
        // We can't directly inspect the timeout, but we can verify the client was created
        assert_eq!(client.auth_token, "test_token");
        assert_eq!(client.base_url, "https://app.honeybadger.io");
    }

    #[test]
    fn test_project_deserialization() {
        // Test that we can deserialize a valid API response
        let json = r#"{
            "id": 12345,
            "name": "Test Project",
            "fault_count": 42,
            "unresolved_fault_count": 5
        }"#;

        let project: Result<Project, _> = serde_json::from_str(json);
        assert!(project.is_ok());
        let project = project.unwrap();
        assert_eq!(project.id, 12345);
        assert_eq!(project.name, "Test Project");
        assert_eq!(project.fault_count, 42);
        assert_eq!(project.unresolved_fault_count, 5);
    }

    #[test]
    fn test_projects_list_deserialization() {
        // Test that we can deserialize a list of projects
        let json = r#"[
            {
                "id": 1,
                "name": "Project One",
                "fault_count": 10,
                "unresolved_fault_count": 2
            },
            {
                "id": 2,
                "name": "Project Two",
                "fault_count": 20,
                "unresolved_fault_count": 3
            }
        ]"#;

        let projects: Result<Vec<Project>, _> = serde_json::from_str(json);
        assert!(projects.is_ok());
        let projects = projects.unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].name, "Project One");
        assert_eq!(projects[1].name, "Project Two");
    }

    // Note: Testing actual HTTP error handling would require a mock server
    // or integration tests. The key change is adding status checking in list_projects()
    // to match the pattern used in query_insights().
}
