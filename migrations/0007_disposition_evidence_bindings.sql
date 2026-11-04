ALTER TABLE agent_tool_calls
    ADD CONSTRAINT agent_tool_calls_id_run_idx UNIQUE (id, run_id);

ALTER TABLE disposition_proposals
    ADD CONSTRAINT disposition_proposals_id_run_idx UNIQUE (id, source_run_id);

ALTER TABLE disposition_proposal_evidence
    ADD COLUMN source_run_id UUID NOT NULL,
    ADD COLUMN agent_tool_call_id UUID NOT NULL,
    ADD CONSTRAINT disposition_evidence_proposal_run_fk
        FOREIGN KEY (proposal_id, source_run_id)
        REFERENCES disposition_proposals(id, source_run_id)
        ON DELETE CASCADE,
    ADD CONSTRAINT disposition_evidence_tool_call_run_fk
        FOREIGN KEY (agent_tool_call_id, source_run_id)
        REFERENCES agent_tool_calls(id, run_id),
    ADD CONSTRAINT disposition_evidence_id_matches_call
        CHECK (evidence_id = agent_tool_call_id::TEXT);
