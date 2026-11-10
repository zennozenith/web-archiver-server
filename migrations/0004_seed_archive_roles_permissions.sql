-- =============================================================================
-- SEED: archive permissions, wired into the roles..
-- (super_admin already has '*'.)
-- =============================================================================
INSERT INTO permissions (name, description) VALUES
    ('archive:read',  'View own archive (pages, captures, collections, tags)'),
    ('archive:write', 'Save, edit and delete own pages, captures, collections and tags'),
    ('archive:admin', 'View and manage any user''s archive');

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r, permissions p
 WHERE (r.name = 'admin'     AND p.name IN ('archive:read','archive:write','archive:admin'))
    OR (r.name = 'moderator' AND p.name IN ('archive:read'))
    OR (r.name = 'user'      AND p.name IN ('archive:read','archive:write'));

