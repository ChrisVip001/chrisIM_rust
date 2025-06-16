alter table public.users
    alter column avatar_url type varchar(1000) using avatar_url::varchar(1000);