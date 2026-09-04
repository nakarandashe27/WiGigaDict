CREATE UNIQUE INDEX ux_delivery_operation_user_action
    ON delivery_operation (user_action_id)
    WHERE user_action_id IS NOT NULL;
