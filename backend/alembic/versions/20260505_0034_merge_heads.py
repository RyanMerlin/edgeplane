"""merge evolve_run_score and service_accounts heads

Revision ID: jjj0505001
Revises: iii0424001, iii0503001
Create Date: 2026-05-05

"""

from alembic import op
import sqlalchemy as sa

revision = "jjj0505001"
down_revision = ("iii0424001", "iii0503001")
branch_labels = None
depends_on = None


def upgrade() -> None:
    pass


def downgrade() -> None:
    pass
