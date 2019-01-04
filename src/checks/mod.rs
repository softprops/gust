//! Checks interface
// see: https://developer.github.com/v3/checks/suites/
use futures::IntoFuture;
use hyper::client::connect::Connect;
use serde::{Deserialize, Serialize};

use self::super::{AuthenticationConstraint, Future, Github, MediaType};

pub struct CheckRuns<C>
where
    C: Clone + Connect + 'static,
{
    github: Github<C>,
    owner: String,
    repo: String,
}

impl<'a, C> CheckRuns<C>
where
    C: Clone + Connect + 'static,
{
    #[doc(hidden)]
    pub(crate) fn new<O, R>(github: Github<C>, owner: O, repo: R) -> Self
    where
        O: Into<String>,
        R: Into<String>,
    {
        CheckRuns {
            github: github,
            owner: owner.into(),
            repo: repo.into(),
        }
    }

    fn path(&self, more: &str) -> String {
        format!("/repos/{}/{}/check-runs{}", self.owner, self.repo, more)
    }

    pub fn create(&self, check_run_options: &CheckRunOptions) -> Future<CheckRun> {
        match serde_json::to_string(check_run_options) {
            Ok(data) => self.github.post_media::<CheckRun>(
                &self.path(""),
                data.into_bytes(),
                MediaType::Preview("antiope"),
                AuthenticationConstraint::Unconstrained,
            ),
            Err(e) => Box::new(Err(e.into()).into_future()),
        }
    }

    pub fn update(
        &self,
        check_run_id: &str,
        check_run_options: &CheckRunUpdateOptions,
    ) -> Future<CheckRun> {
        match serde_json::to_string(check_run_options) {
            Ok(data) => self.github.post_media::<CheckRun>(
                &self.path(&format!("/{}", check_run_id)),
                data.into_bytes(),
                MediaType::Preview("antiope"),
                AuthenticationConstraint::Unconstrained,
            ),
            Err(e) => Box::new(Err(e.into()).into_future()),
        }
    }

    pub fn list_for_suite(&self, suite_id: &str) -> Future<Vec<CheckRun>> {
        // !!! does this actually work?
        // https://developer.github.com/v3/checks/runs/#list-check-runs-in-a-check-suite
        self.github.get_media::<Vec<CheckRun>>(
            &self.path(&format!("/{}/check-runs", suite_id)),
            MediaType::Preview("antiope"),
        )
    }
}

// representations

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CheckRunState {
    Queued,
    InProgress,
    Completed,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Conclusion {
    Success,
    Failure,
    Neutral,
    Cancelled,
    TimedOut,
    ActionRequired,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationLevel {
    Notice,
    Warning,
    Failure,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Output {
    pub title: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<Image>>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Action {
    pub label: String,
    pub description: String,
    pub identifier: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Annotation {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_column: Option<u32>,
    pub annotation_level: AnnotationLevel,
    pub message: String,
    pub title: String,
    pub raw_details: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Image {
    pub alt: String,
    pub image_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CheckRunOptions {
    pub name: String,
    pub head_sha: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CheckRunState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<Conclusion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Output>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<Action>>,
}


#[derive(Debug, Serialize, PartialEq)]
pub struct CheckRunUpdateOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CheckRunState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<Conclusion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Output>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<Action>>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CheckRun {
    pub id: i32,
    pub name: String,
    pub head_sha: String,
    pub url: String,
    pub check_suite: CheckSuite,
    pub details_url: Option<String>,
    pub external_id: Option<String>,
    pub status: Option<CheckRunState>,
    pub started_at: Option<String>,
    pub conclusion: Option<Conclusion>,
    pub completed_at: Option<String>,
    /*
    Deleted for now:

    GitHub's API returns:

      "output": {
        "title": null,
        "summary": null,
        "text": null,
        "annotations_count": 0,
        "annotations_url": "https://api.github.com/repos/grahamc/notpkgs/check-runs/30726963/annotations"
      },

    if there is no Output, which confuses serde.


    pub output: Option<Output>,
     */
    pub actions: Option<Vec<Action>>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CheckSuite {
    pub id: u32,
}
