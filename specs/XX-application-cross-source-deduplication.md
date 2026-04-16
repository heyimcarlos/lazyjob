# Spec: Cross-Source Application Deduplication

## Context

Same job appears on multiple platforms (LinkedIn, Greenhouse, company careers page) with different IDs. LazyJob currently treats these as separate applications, inflating pipeline metrics and causing wasted effort.

## Motivation

- **Metrics accuracy**: Duplicate applications distort interview rates, conversion funnels
- **User efficiency**: Don't tailor 3 resumes for same job
- **Data quality**: Clean view of actual application count

## Design

### Job Fingerprinting

Jobs are matched using a composite fingerprint:

```rust
pub struct JobFingerprint {
    pub normalized_company: String,      // "stripe" (lowercase, stripped Inc/LLC)
    pub normalized_title: String,        // "senior software engineer" (normalized)
    pub location_match: bool,             // Same city/region
    pub description_similarity: f32,      // 0.0 - 1.0 TF-IDF cosine similarity
    pub posted_date_delta: Option<i64>,   // Days apart if known
}

impl JobFingerprint {
    /// Generate fingerprint from job data
    pub fn generate(job: &Job) -> Self { /* ... */ }
    
    /// Calculate similarity between two fingerprints
    pub fn similarity(&self, other: &JobFingerprint) -> f32 {
        // Company match (exact after normalization)
        let company_match = self.normalized_company == other.normalized_company;
        
        // Title similarity (cosine of normalized tokens)
        let title_sim = cosine_similarity(&self.normalized_title, &other.normalized_title);
        
        // Location match
        let location_match = self.location_match == other.location_match;
        
        // Weighted score
        let company_score = if company_match { 1.0 } else { 0.0 };
        let title_score = title_sim * 0.7;  // Title weighted less than company
        
        (company_score * 0.5 + title_score * 0.4 + location_match as f32 * 0.1).min(1.0)
    }
}
```

### Deduplication Matching

```rust
pub struct DeduplicationService {
    threshold: f32,  // Default 0.85 - above this = same job
}

impl DeduplicationService {
    /// Find duplicate jobs in a list
    pub fn find_duplicates(&self, jobs: &[Job]) -> Vec<Vec<JobId>> {
        let mut groups: Vec<Vec<JobId>> = Vec::new();
        let mut processed: HashSet<JobId> = HashSet::new();
        
        for job in jobs {
            if processed.contains(&job.id) {
                continue;
            }
            
            let fp = JobFingerprint::generate(job);
            let mut group = vec![job.id];
            processed.insert(job.id);
            
            // Find all matches
            for other in jobs {
                if processed.contains(&other.id) {
                    continue;
                }
                
                let other_fp = JobFingerprint::generate(other);
                if fp.similarity(&other_fp) >= self.threshold {
                    group.push(other.id);
                    processed.insert(other.id);
                }
            }
            
            if group.len() > 1 {
                groups.push(group);
            }
        }
        
        groups
    }
}
```

### Application Consolidation

When same job is found from multiple sources:

```rust
pub enum ConsolidationStrategy {
    /// Keep all applications but mark as duplicates (user sees them all)
    KeepSeparate,
    /// Merge into single application, link all source jobs
    MergeToPrimary { primary_source: Source },
    /// User decides for each case
    AskUser,
}

pub struct ConsolidatedJob {
    pub primary_job_id: JobId,
    pub source_jobs: Vec<SourceJob>,  // All sources
    pub consolidated_application_id: Option<ApplicationId>,
}
```

### UI for Duplicates

When duplicates detected, TUI shows:

```
┌─────────────────────────────────────────────────────────────┐
│ ⚠️ 3 applications may be for the same job                    │
│                                                             │
│ [View Details] [Keep Separate] [Merge to One]               │
└─────────────────────────────────────────────────────────────┘
```

### Source Priority

When merging data from multiple sources, priority order:

1. **LinkedIn**: Best job descriptions, salary data
2. **Greenhouse**: Structured data, interview process info
3. **Lever**: Company culture, hiring process
4. **Direct careers**: Most accurate job details

```rust
pub fn resolve_field<'a>(field: &str, sources: &[(Source, &'a str)]) -> &'a str {
    for (source, value) in sources {
        if !value.is_empty() {
            return value;
        }
    }
    sources[0].1  // Fallback to first source
}
```

## Implementation Notes

- Deduplication runs on job import (before adding to database)
- Groups stored in `job_duplicate_groups` table
- User can override automatic matching
- Periodic re-check for existing jobs (in case new source added)

## Open Questions

1. **False positives**: Can two different jobs have similar fingerprints?
2. **Same company, different roles**: Handle carefully ("Engineering Manager" vs "Product Manager")
3. **Auto-merge default**: Should auto-merge or ask user?

## Related Specs

- `application-workflow-actions.md` - Application creation
- `job-search-discovery-engine.md` - Cross-source deduplication
- `application-state-machine.md` - Application records