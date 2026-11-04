ALTER TABLE action_plans
    ADD COLUMN source_evidence_state TEXT CHECK (
        source_evidence_state IS NULL OR source_evidence_state IN (
            'supported', 'suspected_mismatch', 'inconclusive',
            'confirmed_match', 'confirmed_mismatch'
        )
    ),
    ADD COLUMN compiler_policy_version TEXT CHECK (
        compiler_policy_version IS NULL OR
        char_length(compiler_policy_version) BETWEEN 1 AND 64
    ),
    ADD COLUMN compiled_at TIMESTAMPTZ,
    ADD CONSTRAINT action_plans_compiler_metadata_complete CHECK (
        (
            source_evidence_state IS NULL AND
            compiler_policy_version IS NULL AND
            compiled_at IS NULL
        ) OR (
            source_evidence_state IS NOT NULL AND
            compiler_policy_version IS NOT NULL AND
            compiled_at IS NOT NULL
        )
    );
