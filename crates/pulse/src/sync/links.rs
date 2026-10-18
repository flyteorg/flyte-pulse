//! Choose the Git range that lists a pull request's commits.
//!
//! These rules reproduced the GitHub REST pull request commit lists for 1618/1620
//! flyte-sdk and 3112/3114 flytekit pull requests; the rest had newer pushes or
//! exceeded the REST API's 250-commit limit.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkPlan {
    /// Merged with a merge commit: commits reachable from the head but not the
    /// merge's first parent.
    MergeRange { head: String, base: String },
    /// Squash, rebase, open, or closed: commits on the pull request ref that no
    /// tracked target branch contains.
    PullRange { number: u64 },
}

pub struct LinkInputs<'a> {
    pub number: u64,
    pub head_sha: &'a str,
    pub merge_commit_sha: &'a str,
    /// Local `refs/pull/<N>/head`, when fetched.
    pub pull_head: Option<&'a str>,
}

/// `parents` returns a local commit's parents; `first_parent_merges` maps a
/// merge's second parent to `(merge, first parent)` on the target branches.
pub fn plan_links(
    inputs: &LinkInputs<'_>,
    parents: impl Fn(&str) -> Option<Vec<String>>,
    first_parent_merges: &HashMap<String, (String, String)>,
) -> Option<LinkPlan> {
    let head = if !inputs.head_sha.is_empty() { Some(inputs.head_sha) } else { inputs.pull_head };
    if !inputs.merge_commit_sha.is_empty()
        && let Some(merge_parents) = parents(inputs.merge_commit_sha)
        && merge_parents.len() >= 2
    {
        let head = head.unwrap_or(&merge_parents[1]).to_string();
        return Some(LinkPlan::MergeRange { head, base: merge_parents[0].clone() });
    }
    for candidate in [inputs.head_sha, inputs.pull_head.unwrap_or("")] {
        if candidate.is_empty() {
            continue;
        }
        if let Some((_, first_parent)) = first_parent_merges.get(candidate) {
            return Some(LinkPlan::MergeRange { head: candidate.to_string(), base: first_parent.clone() });
        }
    }
    inputs.pull_head.map(|_| LinkPlan::PullRange { number: inputs.number })
}

/// `git rev-list` arguments for a plan.
pub fn rev_list_args(plan: &LinkPlan, branches: &[String]) -> Vec<String> {
    match plan {
        LinkPlan::MergeRange { head, base } => vec![head.clone(), "--not".into(), base.clone()],
        LinkPlan::PullRange { number } => {
            let mut args = vec![format!("refs/pull/{number}/head"), "--not".into()];
            args.extend(branches.iter().map(|b| format!("refs/heads/{b}")));
            args
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parents_of(map: &[(&str, &[&str])]) -> impl Fn(&str) -> Option<Vec<String>> {
        let map: HashMap<String, Vec<String>> =
            map.iter().map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect())).collect();
        move |sha| map.get(sha).cloned()
    }

    #[test]
    fn merge_commit_uses_first_parent_as_base() {
        let inputs = LinkInputs { number: 5, head_sha: "head", merge_commit_sha: "m", pull_head: Some("head") };
        let plan = plan_links(&inputs, parents_of(&[("m", &["p1", "head"])]), &HashMap::new());
        assert_eq!(plan, Some(LinkPlan::MergeRange { head: "head".into(), base: "p1".into() }));
        assert_eq!(rev_list_args(&plan.unwrap(), &["main".into()]), vec!["head", "--not", "p1"]);
    }

    #[test]
    fn squash_merge_uses_pull_ref() {
        let inputs = LinkInputs { number: 9, head_sha: "head", merge_commit_sha: "squash", pull_head: Some("head") };
        let plan = plan_links(&inputs, parents_of(&[("squash", &["p1"])]), &HashMap::new()).unwrap();
        assert_eq!(plan, LinkPlan::PullRange { number: 9 });
        assert_eq!(
            rev_list_args(&plan, &["main".into(), "master".into()]),
            vec!["refs/pull/9/head", "--not", "refs/heads/main", "refs/heads/master"]
        );
    }

    #[test]
    fn seeded_merge_found_from_first_parent_history() {
        let merges = HashMap::from([("head".to_string(), ("m".to_string(), "p1".to_string()))]);
        let inputs = LinkInputs { number: 3, head_sha: "head", merge_commit_sha: "", pull_head: Some("head") };
        let plan = plan_links(&inputs, |_| None, &merges);
        assert_eq!(plan, Some(LinkPlan::MergeRange { head: "head".into(), base: "p1".into() }));
    }

    #[test]
    fn no_ref_and_no_merge_means_no_plan() {
        let inputs = LinkInputs { number: 3, head_sha: "head", merge_commit_sha: "", pull_head: None };
        assert_eq!(plan_links(&inputs, |_| None, &HashMap::new()), None);
    }
}
