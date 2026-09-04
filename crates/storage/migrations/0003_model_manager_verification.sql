ALTER TABLE model_package
    ADD COLUMN signature_key_id TEXT
        CHECK (signature_key_id IS NULL OR length(signature_key_id) > 0);

ALTER TABLE model_package
    ADD COLUMN release_sequence INTEGER
        CHECK (release_sequence IS NULL OR release_sequence > 0);

ALTER TABLE model_package
    ADD COLUMN compatibility_abi TEXT
        CHECK (compatibility_abi IS NULL OR length(compatibility_abi) > 0);
